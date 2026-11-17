//! Functions for rapifying and derapifying Arma configs

use std::cmp::min;
use std::collections::HashSet;
use std::io::{BufReader, BufWriter, Cursor, Error, Read, Seek, SeekFrom, Write};
use std::iter::Sum;
use std::path::PathBuf;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::error;
use crate::error::{ConfigParseErrorExt, ErrorExt, located_error, printable};
use crate::eval::{EvalValue, eval, exec};
use crate::io::{ReadExt, WriteExt, compressed_int_len};
use crate::preprocess::{IncludeFolders, MAX_NESTING_DEPTH, nesting_error, preprocess_in};
use crate::warning::{Warning, WarningSink};

/// Refuses a name with a control character: the config grammar has no way to write one, and it
/// would reach the terminal raw.
fn text_name(name: &str, what: &str) -> Result<(), Error> {
    if name.chars().any(char::is_control) {
        return Err(error!(
            "{what} name \"{}\" cannot be written as config text.",
            printable(name)
        ));
    }
    Ok(())
}

/// State shared with the grammar for `__EVAL` / `__EXEC`.
#[derive(Default)]
pub(crate) struct EvalCtx {
    vars: std::collections::HashMap<String, EvalValue>,
    /// The first error raised from a grammar action, with the byte offset of the token it belongs to.
    error: Option<(usize, Error)>,
}

impl From<EvalValue> for ConfigEntry {
    fn from(v: EvalValue) -> ConfigEntry {
        match v {
            EvalValue::Str(s) => ConfigEntry::StringEntry(s),
            EvalValue::Number(n)
                if matches!(n.fract().classify(), std::num::FpCategory::Zero)
                    && n.abs() < 2_147_483_648.0 =>
            {
                // the guard above ensures the value fits an i32 without truncation
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "guarded by fract() == 0 and abs() < 2^31"
                )]
                {
                    ConfigEntry::IntEntry(n as i32)
                }
            }
            EvalValue::Number(n) => ConfigEntry::FloatEntry(n),
        }
    }
}

impl From<EvalValue> for ConfigArrayElement {
    fn from(v: EvalValue) -> ConfigArrayElement {
        match v {
            EvalValue::Str(s) => ConfigArrayElement::StringElement(s),
            EvalValue::Number(n)
                if matches!(n.fract().classify(), std::num::FpCategory::Zero)
                    && n.abs() < 2_147_483_648.0 =>
            {
                // the guard above ensures the value fits an i32 without truncation
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "guarded by fract() == 0 and abs() < 2^31"
                )]
                {
                    ConfigArrayElement::IntElement(n as i32)
                }
            }
            EvalValue::Number(n) => ConfigArrayElement::FloatElement(n),
        }
    }
}

/// Warnings raised while parsing a config, with the byte offset they occurred at.
pub(crate) struct ConfigWarnings(pub(crate) Vec<(usize, String, Option<&'static str>)>);

impl ConfigWarnings {
    fn push(&mut self, warning: (usize, String, Option<&'static str>)) {
        self.0.push(warning);
    }
}

peg::parser! {
    /// PEG grammar for rapified config syntax.
    pub(crate) grammar config_grammar(warnings: &mut ConfigWarnings, ctx: &mut EvalCtx) for str {
        rule whitespace() = quiet!{ [' ' | '\r' | '\n' | '\t']+ }

        rule exponent() = ['e' | 'E'] ['-' | '+']? ['0'..='9']+

        rule float_text() -> (usize, &'input str) = p:position!() t:$((['-' | '+'])? ((['0'..='9']* "." ['0'..='9']+ exponent()?) / (['0'..='9']+ exponent()))) { (p, t) }

        /// The end of an entry: `;`, the `}` of the class, or (for the last entry of a block or file,
        /// as CfgConvert reads it) the end of the line.
        rule entry_end() = whitespace()? [';' | '}'] / [' ' | '\t']* ['\r' | '\n']

        /// A float followed by the end of an entry. The terminator is checked before the value is,
        /// because an error raised for `1e999x` would stay and beat the unquoted-string fallback.
        rule float_entry() -> f32 = f:float_text() &entry_end() v:float_value(f) { v }

        /// A float followed by the end of an array element.
        rule float_element() -> f32 = f:float_text() &(whitespace()? [',' | '}']) v:float_value(f) { v }

        rule float_value(at: (usize, &str)) -> f32 = {?
            let (pos, f) = at;
            // `parse` gives infinity on overflow; that would not survive a derapify/rapify round trip
            f.parse::<f32>()
                .map_err(|e| { ctx.error.get_or_insert((pos, error!("Invalid float \"{f}\": {e}"))); "a valid float" })
                .and_then(|v| if v.is_finite() { Ok(v) } else {
                    ctx.error.get_or_insert((pos, error!("Invalid float \"{f}\": out of range")));
                    Err("a float in range")
                })
        }

        rule integer_text() -> (usize, &'input str) = p:position!() t:$((['-' | '+'])? (("0x" ['0'..='9' | 'a'..='f' | 'A'..='F']+) / ['0'..='9']+)) { (p, t) }

        /// An integer followed by the end of an entry; the terminator is checked first, like for floats
        /// (`99999999999x` stays an unquoted string).
        rule integer_entry() -> i32 = i:integer_text() &entry_end() v:integer_value(i) { v }

        /// An integer followed by the end of an array element.
        rule integer_element() -> i32 = i:integer_text() &(whitespace()? [',' | '}']) v:integer_value(i) { v }

        rule integer_value(at: (usize, &str)) -> i32 = {?
            let (pos, i) = at;
            if i.contains("0x") {
                i32::from_str_radix(&i.replace("0x", ""), 16)
                    .map_err(|e| { ctx.error.get_or_insert((pos, error!("Invalid integer \"{i}\": {e}"))); "a valid integer" })
            } else {
                i.parse::<i32>()
                    .map_err(|e| { ctx.error.get_or_insert((pos, error!("Invalid integer \"{i}\": {e}"))); "a valid integer" })
            }
        }

        rule integer() -> i32 = i:integer_text() v:integer_value(i) { v }

        rule doublequoted_string() -> String = "\"" s:$(("\"\"" / !['"'] [_])*) "\"" {
            s.to_string().replace("\"\"", "\"")
        }

        rule singlequoted_string() -> String = "'" s:$(("''" / !['\''] [_])*) "'" {
            s.to_string().replace("''", "'")
        }

        rule unquoted_string() -> String = pos:position!() s:$((![';' | '}' | '\r' | '\n'] [_])*) {
            let result = s.to_string().trim().to_string();
            // an empty value comes from a macro that expanded to nothing (e.g. an argument-count
            // mismatch, which has its own warning); CfgConvert makes it "" as well.
            // `unquoted_string_entry` raises `empty-value` for it
            if !result.is_empty() {
                warnings.push((pos, format!("String value \"{}\" is not quoted properly.", printable(&result)), Some("unquoted-string")));
            }
            result
        }

        rule unquoted_string_array() -> String = pos:position!() s:$( (![',' | '}' | ' ' | '\t' | '\r' | '\n'] [_]) (![',' | '}' | '\r' | '\n'] [_])* ) {
            let result = s.to_string().trim().to_string();
            warnings.push((pos, format!("String array element \"{}\" is not quoted properly.", printable(&result)), Some("unquoted-string")));
            result
        }

        /// Fails (and records the error) when `d` levels of classes, arrays or parentheses are exceeded.
        rule nesting_ok(d: usize) = pos:position!() {?
            if d >= MAX_NESTING_DEPTH {
                ctx.error.get_or_insert((pos, nesting_error()));
                Err("nesting within the limit")
            } else {
                Ok(())
            }
        }

        rule eval_body(d: usize) = ("(" nesting_ok(d) eval_body(d + 1) ")" / "\"" (!"\"" [_])* "\"" / !['(' | ')'] [_])*

        rule eval_value(d: usize) -> EvalValue = pos:position!() "__EVAL" whitespace()? "(" b:$(eval_body(d)) ")" {?
            match eval(b, &mut ctx.vars) {
                Ok(v) => Ok(v),
                Err(e) => { ctx.error.get_or_insert((pos, e)); Err("a supported __EVAL expression") }
            }
        }

        rule exec_entry(d: usize) = pos:position!() "__EXEC" whitespace()? "(" b:$(eval_body(d)) ")" whitespace()? ";" {?
            match exec(b, &mut ctx.vars) {
                Ok(()) => Ok(()),
                Err(e) => { ctx.error.get_or_insert((pos, e)); Err("a supported __EXEC statement") }
            }
        }

        rule string() -> String = doublequoted_string() / singlequoted_string()

        rule array_element(d: usize) -> ConfigArrayElement =
            f:float_element() { ConfigArrayElement::FloatElement(f) } /
            i:integer_element() { ConfigArrayElement::IntElement(i) } /
            a:array(d)   &(whitespace()? [',' | '}']) { ConfigArrayElement::ArrayElement(a) } /
            s:string()  &(whitespace()? [',' | '}']) { ConfigArrayElement::StringElement(s) } /
            v:eval_value(d) &(whitespace()? [',' | '}']) { v.into() } /
            s:unquoted_string_array() &(whitespace()? [',' | '}']) { ConfigArrayElement::StringElement(s) }

        rule array_elements(d: usize) -> Vec<ConfigArrayElement> = array_element(d) ** (whitespace()? "," whitespace()?)

        rule array(d: usize) -> ConfigArray = "{" nesting_ok(d) whitespace()? elems:array_elements(d + 1) whitespace()? ","? whitespace()? "}" {
            ConfigArray {
                is_expansion: false,
                elements: elems
            }
        }

        rule var(d: usize) -> ConfigEntry =
            f:float_entry() { ConfigEntry::FloatEntry(f) } /
            i:integer_entry() { ConfigEntry::IntEntry(i) } /
            s:string()  { ConfigEntry::StringEntry(s) } /
            v:eval_value(d) { v.into() }

        rule var_entry(d: usize) -> (String, ConfigEntry) = n:name() whitespace()? "=" whitespace()? ce:var(d) {
            (n, ce)
        }

        rule unquoted_string_entry() -> (String, ConfigEntry) = pos:position!() n:name() whitespace()? "=" whitespace()? s:unquoted_string() {
            if s.is_empty() {
                // `x = ;`, or a macro that expanded to nothing; kept as "" so existing mods still build
                warnings.push((pos, format!("Entry \"{}\" has no value; it is stored as an empty string.", printable(&n)), Some("empty-value")));
            }
            (n, ConfigEntry::StringEntry(s))
        }

        rule array_entry(d: usize) -> (String, ConfigEntry) = n:name() whitespace()? "[" whitespace()? "]" whitespace()? "=" whitespace()? a:array(d) {
            (n, ConfigEntry::ArrayEntry(a))
        }

        rule array_expansion_entry(d: usize) -> (String, ConfigEntry) = n:name() whitespace()? "[" whitespace()? "]" whitespace()? "+=" whitespace()? a:array(d) {
            (n, ConfigEntry::ArrayEntry(ConfigArray {
                is_expansion: true,
                ..a
            }))
        }

        rule enum_value() -> (String, Option<i32>) = n:name() v:(whitespace()? "=" whitespace()? i:integer() { i })? { (n, v) }

        /// `enum { name [= int], ... }`: values count up from the last explicit one, names are stored lowercase.
        rule enum_block() -> Vec<(String, i32)> = quiet!{"enum"} whitespace()? "{" whitespace()? v:(enum_value() ** (whitespace()? "," whitespace()?)) whitespace()? ","? whitespace()? "}" {?
            let mut next: Option<i32> = Some(0);
            let mut out = Vec::new();
            for (n, v) in v {
                let value = match v {
                    Some(value) => value,
                    None => next.ok_or("an enum value that fits in 32 bits")?,
                };
                next = value.checked_add(1);
                out.push((n.to_ascii_lowercase(), value));
            }
            Ok(out)
        }

        rule array_item(d: usize) -> Item =
            e:(array_entry(d) / array_expansion_entry(d)) whitespace()? (";" / &"}") { Item::Entry(e.0, e.1, Vec::new()) }

        /// A value entry; the last one of a block or file may end at the line end instead of `;`.
        rule var_item(d: usize) -> Item =
            e:var_entry(d) (whitespace()? (";" / &"}") / [' ' | '\t']* &['\r' | '\n']) { Item::Entry(e.0, e.1, Vec::new()) }

        rule entry(d: usize) -> Item =
            e:enum_block() whitespace()? (";" / &"}") { Item::Enums(e) } /
            e:class(d) whitespace()? (";" / &"}") { e } /
            e:(array_item(d) / var_item(d)) { e } /
            e:unquoted_string_entry() (";" / &"}" / &['\r' | '\n']) { Item::Entry(e.0, e.1, Vec::new()) }

        rule entry_or_exec(d: usize) -> Item = exec_entry(d) { Item::Nothing } / entry(d)

        rule entries(d: usize) -> (Vec<(String, ConfigEntry)>, Vec<(String, i32)>) = v:(entry_or_exec(d) ** (whitespace()?)) {
            let mut entries = Vec::new();
            let mut enums = Vec::new();
            for item in v {
                match item {
                    Item::Entry(n, e, en) => { entries.push((n, e)); enums.extend(en); }
                    Item::Enums(en) => enums.extend(en),
                    Item::Nothing => {}
                }
            }
            (entries, enums)
        }

        rule name() -> String = n:$(['a'..='z' | 'A'..='Z' | '0'..='9' | '_']+) {
            n.to_string()
        }

        rule parent() -> String = whitespace()? ":" whitespace()? n:name() {
            n
        }

        rule regular_class(d: usize) -> Item = "class" whitespace()+ n:name() p:parent()? whitespace()? "{" nesting_ok(d) whitespace()? e:entries(d + 1) whitespace()? "}" {
            let parent = p.unwrap_or_default();
            Item::Entry(n, ConfigEntry::ClassEntry(ConfigClass {
                parent,
                is_external: false,
                is_deletion: false,
                entries: Some(e.0)
            }), e.1)
        }

        rule external_class() -> Item = "class" whitespace()+ n:name() {
            Item::Entry(n, ConfigEntry::ClassEntry(ConfigClass {
                parent: String::new(),
                is_external: true,
                is_deletion: false,
                entries: None
            }), Vec::new())
        }

        rule deleted_class() -> Item = "delete" whitespace()+ n:name() {
            Item::Entry(n, ConfigEntry::ClassEntry(ConfigClass {
                parent: String::new(),
                is_external: false,
                is_deletion: true,
                entries: None
            }), Vec::new())
        }

        rule class(d: usize) -> Item = regular_class(d) / external_class() / deleted_class()

        /// Parses a complete config: a sequence of entries until end of input.
        pub rule config() -> Config = whitespace()? e:entries(0) whitespace()? ![_] {
            Config {
                root_body: ConfigClass {
                    parent: String::new(),
                    is_external: false,
                    is_deletion: false,
                    entries: Some(e.0)
                },
                enums: e.1,
            }
        }
    }
}

/// Reads the enum table at the header's `enum_offset`. The class tree is usable without it, so a bad
/// offset, a cut-short table or a name that is not a config identifier gives the values read before
/// it, never an error.
fn read_enum_table<I: Read + Seek>(input: &mut I) -> Vec<(String, i32)> {
    let mut out = Vec::new();
    // an unreadable table is not an error: the values read so far are returned
    let _ = (|| -> std::io::Result<()> {
        input.seek(SeekFrom::Start(12))?;
        let offset = input.read_u32::<LittleEndian>()?;
        input.seek(SeekFrom::Start(offset.into()))?;
        let count = input.read_u32::<LittleEndian>()?;
        // no preallocation from the count: the loop ends at the first read error
        for _ in 0..count {
            let name = input.read_cstring()?;
            let value = input.read_i32::<LittleEndian>()?;
            // the table ends at the first name the config grammar cannot write
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                break;
            }
            out.push((name, value));
        }
        Ok(())
    })();
    out
}

/// Returns true if `data` starts with the rapified config signature.
#[must_use]
pub fn is_rapified(data: &[u8]) -> bool {
    data.starts_with(b"\0raP")
}

/// `compressed_int_len` for an in-memory length. Config element counts beyond
/// `u32::MAX` are impossible in practice (each element costs far more than a
/// byte of memory), so they clamp to the maximum; the write path reports them
/// as errors instead.
fn compressed_int_len_of(len: usize) -> usize {
    compressed_int_len(u32::try_from(len).unwrap_or(u32::MAX))
}

/// Config
///
/// # Examples
///
/// ```
/// # use dayzmake_core::config::Config;
/// # use dayzmake_core::warning::Warnings;
/// let input = String::from("
/// #define bar 42
///
/// foo = bar;
/// ");
///
/// let config = Config::from_string(input, None, &Vec::new(), &mut Warnings::new()).expect("Failed to parse config");
///
/// assert_eq!("foo = 42;\n", config.to_string().unwrap());
/// assert_eq!(b"\0raP", &config.to_cursor().unwrap().into_inner()[..4]);
/// ```
#[derive(Debug)]
pub struct Config {
    root_body: ConfigClass,
    enums: Vec<(String, i32)>,
}

/// One parsed config item: the grammar returns the enum values of nested blocks upwards.
enum Item {
    Entry(String, ConfigEntry, Vec<(String, i32)>),
    Enums(Vec<(String, i32)>),
    Nothing,
}

/// Config class
#[derive(Debug)]
pub struct ConfigClass {
    parent: String,
    is_external: bool,
    is_deletion: bool,
    entries: Option<Vec<(String, ConfigEntry)>>,
}

/// Config entry
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigEntry {
    /// String entry
    StringEntry(String),
    /// Float entry
    FloatEntry(f32),
    /// Int entry
    IntEntry(i32),
    /// Array entry
    ArrayEntry(ConfigArray),
    /// Class entry
    ClassEntry(ConfigClass),
}

/// Config array
#[derive(Debug)]
pub struct ConfigArray {
    is_expansion: bool,
    elements: Vec<ConfigArrayElement>,
}

/// Config array element
#[derive(Debug)]
#[non_exhaustive]
pub enum ConfigArrayElement {
    /// String element
    StringElement(String),
    /// Float element
    FloatElement(f32),
    /// Int element
    IntElement(i32),
    /// Array element
    ArrayElement(ConfigArray),
}

impl ConfigArrayElement {
    fn rapified_length(&self) -> usize {
        match self {
            ConfigArrayElement::StringElement(s) => s.len() + 2,
            ConfigArrayElement::FloatElement(_f) => 5,
            ConfigArrayElement::IntElement(_i) => 5,
            ConfigArrayElement::ArrayElement(a) => {
                1 + compressed_int_len_of(a.elements.len())
                    + usize::sum(a.elements.iter().map(ConfigArrayElement::rapified_length))
            }
        }
    }
}

/// Deepest class or array nesting accepted when reading a rapified config: the root class plus the
/// `MAX_NESTING_DEPTH` levels the text parser accepts, so every config this crate writes reads back.
const MAX_RAPIFIED_DEPTH: u32 = 129;

/// Most entries and array elements accepted in one rapified config.
const MAX_RAPIFIED_ENTRIES: u64 = 4_000_000;

/// Limits that keep reading a crafted rapified config from running away.
struct ReadBudget {
    depth: u32,
    remaining: u64,
    /// Offsets of the class bodies being read, to catch bodies that contain themselves.
    open_bodies: Vec<u32>,
    /// Offsets of every class body read so far. Real configs never share a body between classes; a file
    /// that does can make the output exponentially larger than the input.
    seen_bodies: HashSet<u32>,
}

impl ReadBudget {
    fn new() -> Self {
        ReadBudget {
            depth: 0,
            remaining: MAX_RAPIFIED_ENTRIES,
            open_bodies: Vec::new(),
            seen_bodies: HashSet::new(),
        }
    }

    fn enter(&mut self) -> Result<(), Error> {
        if self.depth >= MAX_RAPIFIED_DEPTH {
            return Err(error!(
                "Rapified config nesting is deeper than {MAX_RAPIFIED_DEPTH} levels."
            ));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn take_entry(&mut self) -> Result<(), Error> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| error!("Rapified config has too many entries."))?;
        Ok(())
    }
}

impl ConfigArray {
    /// The array's elements.
    #[must_use]
    pub fn elements(&self) -> &[ConfigArrayElement] {
        &self.elements
    }

    /// Whether this is an array expansion (`x[] += {...}`).
    #[must_use]
    pub fn is_expansion(&self) -> bool {
        self.is_expansion
    }

    fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        output.write_all(b"{")?;
        for (key, value) in self.elements.iter().enumerate() {
            match value {
                ConfigArrayElement::ArrayElement(a) => {
                    a.write(output)?;
                }
                ConfigArrayElement::StringElement(s) => {
                    output.write_all(
                        format!(
                            "\"{}\"",
                            s.replace('\r', "\\r")
                                .replace('\n', "\\n")
                                .replace('"', "\"\"")
                        )
                        .as_bytes(),
                    )?;
                }
                ConfigArrayElement::FloatElement(f) => {
                    output.write_all(format!("{f:?}").as_bytes())?;
                }
                ConfigArrayElement::IntElement(i) => {
                    output.write_all(format!("{i}").as_bytes())?;
                }
            }
            if key < self.elements.len() - 1 {
                output.write_all(b", ")?;
            }
        }
        output.write_all(b"}")?;
        Ok(())
    }

    fn write_rapified<O: Write>(&self, output: &mut O) -> Result<usize, Error> {
        let num_elements = u32::try_from(self.elements.len())
            .map_err(|_| error!("too many array elements: {}", self.elements.len()))?;
        let mut written = output.write_compressed_int(num_elements)?;

        for (index, element) in self.elements.iter().enumerate() {
            match element {
                ConfigArrayElement::StringElement(s) => {
                    output.write_all(&[0])?;
                    output
                        .write_cstring(s)
                        .prepend_error(format!("Array element {index}:"))?;
                    written += s.len() + 2;
                }
                ConfigArrayElement::FloatElement(f) => {
                    output.write_all(&[1])?;
                    output.write_f32::<LittleEndian>(*f)?;
                    written += 5;
                }
                ConfigArrayElement::IntElement(i) => {
                    output.write_all(&[2])?;
                    output.write_i32::<LittleEndian>(*i)?;
                    written += 5;
                }
                ConfigArrayElement::ArrayElement(a) => {
                    output.write_all(&[3])?;
                    written += 1 + a
                        .write_rapified(output)
                        .prepend_error(format!("Array element {index}:"))?;
                }
            }
        }

        Ok(written)
    }

    fn read_rapified<I: Read + Seek>(
        input: &mut I,
        budget: &mut ReadBudget,
    ) -> Result<ConfigArray, Error> {
        budget.enter()?;
        let result = ConfigArray::read_rapified_elements(input, budget);
        budget.leave();
        result
    }

    fn read_rapified_elements<I: Read + Seek>(
        input: &mut I,
        budget: &mut ReadBudget,
    ) -> Result<ConfigArray, Error> {
        let num_elements: u32 = input.read_compressed_int()?;
        // never preallocate the full length from untrusted input
        let mut elements: Vec<ConfigArrayElement> =
            Vec::with_capacity(num_elements.min(65536) as usize);

        for _i in 0..num_elements {
            budget.take_entry()?;
            let mut buf = [0u8; 1];
            input.read_exact(&mut buf)?;
            let element_type = buf[0];

            if element_type == 0 {
                elements.push(ConfigArrayElement::StringElement(input.read_cstring()?));
            } else if element_type == 1 {
                elements.push(ConfigArrayElement::FloatElement(
                    input.read_f32::<LittleEndian>()?,
                ));
            } else if element_type == 2 {
                elements.push(ConfigArrayElement::IntElement(
                    input.read_i32::<LittleEndian>()?,
                ));
            } else if element_type == 3 {
                elements.push(ConfigArrayElement::ArrayElement(
                    ConfigArray::read_rapified(input, budget)?,
                ));
            } else {
                return Err(error!("Unrecognized array element type: {}", element_type));
            }
        }

        Ok(ConfigArray {
            is_expansion: false,
            elements,
        })
    }
}

impl ConfigEntry {
    // without the name
    fn rapified_length(&self) -> usize {
        match self {
            ConfigEntry::StringEntry(s) => s.len() + 3,
            ConfigEntry::FloatEntry(_f) => 6,
            ConfigEntry::IntEntry(_i) => 6,
            ConfigEntry::ArrayEntry(a) => {
                let len = 1
                    + compressed_int_len_of(a.elements.len())
                    + usize::sum(a.elements.iter().map(ConfigArrayElement::rapified_length));
                if a.is_expansion { len + 4 } else { len }
            }
            ConfigEntry::ClassEntry(c) => {
                if c.is_external || c.is_deletion {
                    1
                } else {
                    5
                }
            }
        }
    }
}

impl ConfigClass {
    /// The parent class name, or `None` when the class has no parent.
    #[must_use]
    pub fn parent(&self) -> Option<&str> {
        if self.parent.is_empty() {
            None
        } else {
            Some(&self.parent)
        }
    }

    /// Whether this is an external declaration (`class X;`).
    #[must_use]
    pub fn is_external(&self) -> bool {
        self.is_external
    }

    /// Whether this is a deletion (`delete X;`).
    #[must_use]
    pub fn is_deletion(&self) -> bool {
        self.is_deletion
    }

    /// The entries in file order; empty for external classes and deletions.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &ConfigEntry)> {
        self.entries
            .iter()
            .flatten()
            .map(|(name, entry)| (name.as_str(), entry))
    }

    /// Looks up an entry by name, ASCII case-insensitively like the game.
    ///
    /// Returns the first entry with that name; use [`ConfigClass::entries`] to see every entry
    /// when a config defines a name twice. A `delete X;` entry comes back
    /// as [`ConfigEntry::ClassEntry`] with [`ConfigClass::is_deletion`] `true`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ConfigEntry> {
        self.entries()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, entry)| entry)
    }

    fn write<O: Write>(&self, output: &mut O, level: usize, indent: &str) -> Result<(), Error> {
        if let Some(entries) = &self.entries {
            if level > 0 && !entries.is_empty() {
                output.write_all(b"\n")?;
            }
            for (key, value) in entries {
                text_name(key, "Entry")?;
                output.write_all(indent.repeat(level).as_bytes())?;

                match value {
                    ConfigEntry::ClassEntry(c) => {
                        if c.is_deletion {
                            output.write_all(format!("delete {key};\n").as_bytes())?;
                        } else if c.is_external {
                            output.write_all(format!("class {key};\n").as_bytes())?;
                        } else {
                            let parent = if c.parent.is_empty() {
                                String::new()
                            } else {
                                text_name(&c.parent, "Parent class")?;
                                format!(": {}", c.parent)
                            };
                            match &c.entries {
                                Some(entries) => {
                                    if entries.is_empty() {
                                        output.write_all(
                                            format!("class {key}{parent} {{}};\n").as_bytes(),
                                        )?;
                                    } else {
                                        output.write_all(
                                            format!("class {key}{parent} {{").as_bytes(),
                                        )?;
                                        c.write(output, level + 1, indent)?;
                                        output.write_all(indent.repeat(level).as_bytes())?;
                                        output.write_all(b"};\n")?;
                                    }
                                }
                                None => {
                                    output.write_all(
                                        format!("class {key}{parent} {{}};\n").as_bytes(),
                                    )?;
                                }
                            }
                        }
                    }
                    ConfigEntry::StringEntry(s) => {
                        output.write_all(
                            format!(
                                "{} = \"{}\";\n",
                                key,
                                s.replace('\r', "\\r")
                                    .replace('\n', "\\n")
                                    .replace('"', "\"\"")
                            )
                            .as_bytes(),
                        )?;
                    }
                    ConfigEntry::FloatEntry(f) => {
                        output.write_all(format!("{key} = {f:?};\n").as_bytes())?;
                    }
                    ConfigEntry::IntEntry(i) => {
                        output.write_all(format!("{key} = {i};\n").as_bytes())?;
                    }
                    ConfigEntry::ArrayEntry(a) => {
                        if a.is_expansion {
                            output.write_all(format!("{key}[] += ").as_bytes())?;
                        } else {
                            output.write_all(format!("{key}[] = ").as_bytes())?;
                        }
                        a.write(output)?;
                        output.write_all(b";\n")?;
                    }
                }
            }
        }

        Ok(())
    }

    fn rapified_length(&self) -> usize {
        match &self.entries {
            Some(entries) => {
                self.parent.len()
                    + 1
                    + compressed_int_len_of(entries.len())
                    + usize::sum(entries.iter().map(|(k, v)| {
                        k.len()
                            + 1
                            + v.rapified_length()
                            + match v {
                                ConfigEntry::ClassEntry(c) => c.rapified_length(),
                                _ => 0,
                            }
                    }))
            }
            None => 0,
        }
    }

    fn write_rapified<O: Write>(&self, output: &mut O, offset: usize) -> Result<usize, Error> {
        let mut written = 0;

        match &self.entries {
            Some(entries) => {
                output.write_cstring(&self.parent)?;
                written += self.parent.len() + 1;

                let num_entries = u32::try_from(entries.len())
                    .map_err(|_| error!("too many class entries: {}", entries.len()))?;
                written += output.write_compressed_int(num_entries)?;

                let entries_len = usize::sum(
                    entries
                        .iter()
                        .map(|(k, v)| k.len() + 1 + v.rapified_length()),
                );
                let mut class_offset = offset + written + entries_len;
                let mut class_bodies: Vec<Cursor<Box<[u8]>>> = Vec::new();
                let pre_entries = written;

                for (name, entry) in entries {
                    let pre_write = written;
                    match entry {
                        ConfigEntry::StringEntry(s) => {
                            output.write_all(&[1, 0])?;
                            output
                                .write_cstring(name)
                                .and_then(|()| output.write_cstring(s))
                                .prepend_error(format!("Entry \"{}\":", printable(name)))?;
                            written += name.len() + s.len() + 4;
                        }
                        ConfigEntry::FloatEntry(f) => {
                            output.write_all(&[1, 1])?;
                            output.write_cstring(name)?;
                            output.write_f32::<LittleEndian>(*f)?;
                            written += name.len() + 7;
                        }
                        ConfigEntry::IntEntry(i) => {
                            output.write_all(&[1, 2])?;
                            output.write_cstring(name)?;
                            output.write_i32::<LittleEndian>(*i)?;
                            written += name.len() + 7;
                        }
                        ConfigEntry::ArrayEntry(a) => {
                            output.write_all(if a.is_expansion { &[5] } else { &[2] })?;
                            if a.is_expansion {
                                output.write_all(&[1, 0, 0, 0])?;
                                written += 4;
                            }
                            output.write_cstring(name)?;
                            written += name.len()
                                + 2
                                + a.write_rapified(output)
                                    .prepend_error(format!("Entry \"{}\":", printable(name)))?;
                        }
                        ConfigEntry::ClassEntry(c) => {
                            if c.is_external || c.is_deletion {
                                output.write_all(if c.is_deletion { &[4] } else { &[3] })?;
                                output.write_cstring(name)?;
                                written += name.len() + 2;
                            } else {
                                output.write_all(&[0])?;
                                output.write_cstring(name)?;
                                let class_offset_u32 =
                                    u32::try_from(class_offset).map_err(|_| {
                                        error!("class body offset out of range: {class_offset}")
                                    })?;
                                output.write_u32::<LittleEndian>(class_offset_u32)?;
                                written += name.len() + 6;

                                let buffer: Box<[u8]> =
                                    vec![0; c.rapified_length()].into_boxed_slice();
                                let mut cursor: Cursor<Box<[u8]>> = Cursor::new(buffer);
                                class_offset +=
                                    c.write_rapified(&mut cursor, class_offset).prepend_error(
                                        format!("Failed to rapify {}:", printable(name)),
                                    )?;

                                class_bodies.push(cursor);
                            }
                        }
                    }
                    assert_eq!(
                        written - pre_write,
                        entry.rapified_length() + name.len() + 1
                    );
                }

                assert_eq!(written - pre_entries, entries_len);

                for cursor in class_bodies {
                    output.write_all(cursor.get_ref())?;
                    written += cursor.get_ref().len();
                }
            }
            None => {
                return Err(error!("Cannot rapify a class without entries."));
            }
        }

        Ok(written)
    }

    fn read_rapified<I: Read + Seek>(
        input: &mut I,
        level: u32,
        budget: &mut ReadBudget,
    ) -> Result<ConfigClass, Error> {
        let mut fp = 0;
        let body_offset: u32 = if level == 0 {
            input.seek(SeekFrom::Start(16))?;
            16
        } else {
            let classbody_fp: u32 = input.read_u32::<LittleEndian>()?;

            fp = input.stream_position()?;
            input.seek(SeekFrom::Start(classbody_fp.into()))?;
            classbody_fp
        };

        if budget.open_bodies.contains(&body_offset) {
            return Err(error!("Rapified class body cycle at offset {body_offset}."));
        }
        if !budget.seen_bodies.insert(body_offset) {
            return Err(error!(
                "Rapified class body shared at offset {body_offset}."
            ));
        }
        budget.enter()?;
        budget.open_bodies.push(body_offset);
        let result = ConfigClass::read_rapified_body(input, level, budget);
        budget.open_bodies.pop();
        budget.leave();
        let class = result?;

        if level > 0 {
            input.seek(SeekFrom::Start(fp))?;
        }

        Ok(class)
    }

    fn read_rapified_body<I: Read + Seek>(
        input: &mut I,
        level: u32,
        budget: &mut ReadBudget,
    ) -> Result<ConfigClass, Error> {
        let parent = input.read_cstring()?;
        let num_entries: u32 = input.read_compressed_int()?;
        // never preallocate the full length from untrusted input
        let mut entries: Vec<(String, ConfigEntry)> =
            Vec::with_capacity(num_entries.min(65536) as usize);

        for _i in 0..num_entries {
            budget.take_entry()?;
            let mut buf = [0u8; 1];
            input.read_exact(&mut buf)?;
            let entry_type = buf[0];

            if entry_type == 0 {
                let name = input.read_cstring()?;

                let class_entry = ConfigClass::read_rapified(input, level + 1, budget)
                    .prepend_error(format!(
                        "Failed to read rapified class \"{}\":",
                        printable(&name)
                    ))?;
                entries.push((name, ConfigEntry::ClassEntry(class_entry)));
            } else if entry_type == 1 {
                let mut buf = [0u8; 1];
                input.read_exact(&mut buf)?;
                let subtype = buf[0];
                let name = input.read_cstring()?;

                if subtype == 0 {
                    entries.push((name, ConfigEntry::StringEntry(input.read_cstring()?)));
                } else if subtype == 1 {
                    entries.push((
                        name,
                        ConfigEntry::FloatEntry(input.read_f32::<LittleEndian>()?),
                    ));
                } else if subtype == 2 {
                    entries.push((
                        name,
                        ConfigEntry::IntEntry(input.read_i32::<LittleEndian>()?),
                    ));
                } else {
                    return Err(error!("Unrecognized variable entry subtype: {}.", subtype));
                }
            } else if entry_type == 2 || entry_type == 5 {
                if entry_type == 5 {
                    input.seek(SeekFrom::Current(4))?;
                }

                let name = input.read_cstring()?;
                let mut array = ConfigArray::read_rapified(input, budget)
                    .prepend_error("Failed to read rapified array:")?;
                array.is_expansion = entry_type == 5;

                entries.push((name.clone(), ConfigEntry::ArrayEntry(array)));
            } else if entry_type == 3 || entry_type == 4 {
                let name = input.read_cstring()?;
                let class_entry = ConfigClass {
                    parent: String::new(),
                    is_external: entry_type == 3,
                    is_deletion: entry_type == 4,
                    entries: None,
                };

                entries.push((name.clone(), ConfigEntry::ClassEntry(class_entry)));
            } else {
                return Err(error!("Unrecognized class entry type: {}.", entry_type));
            }
        }

        Ok(ConfigClass {
            parent,
            is_external: false,
            is_deletion: false,
            entries: Some(entries),
        })
    }
}

/// Options for [`Config::write_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WriteOptions {
    indent: String,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            indent: "    ".to_string(),
        }
    }
}

impl WriteOptions {
    /// The default options: four spaces of indentation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the string written once per nesting level.
    ///
    /// The string may be empty and may only contain spaces and tabs, at most 64 characters, so the
    /// output stays parsable.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value if it contains other characters or is too long.
    pub fn indent(mut self, indent: impl Into<String>) -> Result<Self, Error> {
        let indent = indent.into();
        let length = indent.chars().count();
        if length > 64 {
            return Err(error!(
                "Invalid indentation: it is {length} characters, at most 64 are allowed."
            ));
        }
        if !indent.chars().all(|c| c == ' ' || c == '\t') {
            return Err(error!(
                "Invalid indentation \"{}\": use spaces and tabs only.",
                printable(&indent)
            ));
        }
        self.indent = indent;
        Ok(self)
    }
}

impl Config {
    /// The unnamed top-level class holding all entries of the file.
    #[must_use]
    pub fn root(&self) -> &ConfigClass {
        &self.root_body
    }

    /// The values of the `enum { ... }` blocks, in source order (or table order for a rapified config).
    ///
    /// Names are lowercase, as the game stores them. The values of all blocks, at any class level,
    /// land in this one table.
    #[must_use]
    pub fn enum_values(&self) -> &[(String, i32)] {
        &self.enums
    }

    fn rapified_enum_table_length(&self) -> usize {
        4 + self
            .enums
            .iter()
            .map(|(n, _)| n.len() + 1 + 4)
            .sum::<usize>()
    }

    /// Writes the config (unrapified) to the output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure.
    pub fn write<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        self.write_with(output, &WriteOptions::new())
    }

    /// Writes the config (unrapified) to the output with the given options.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or if a name cannot be written as text.
    pub fn write_with<O: Write>(
        &self,
        output: &mut O,
        options: &WriteOptions,
    ) -> Result<(), Error> {
        // check everything before the first byte: a block is never half-written
        for (name, _) in &self.enums {
            text_name(name, "Enum")?;
        }
        if !self.enums.is_empty() {
            output.write_all(b"enum {\n")?;
            for (i, (name, value)) in self.enums.iter().enumerate() {
                let comma = if i + 1 < self.enums.len() { "," } else { "" };
                output
                    .write_all(format!("{}{name} = {value}{comma}\n", options.indent).as_bytes())?;
            }
            output.write_all(b"};\n")?;
        }
        self.root_body.write(output, 0, &options.indent)
    }

    /// Returns the unrapified config as a string.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or invalid UTF-8 in the output.
    pub fn to_string(&self) -> Result<String, Error> {
        let buffer = Vec::new();
        let mut cursor: Cursor<Vec<u8>> = Cursor::new(buffer);
        self.write(&mut cursor)?;

        String::from_utf8(cursor.into_inner())
            .map_err(|e| error!("Invalid UTF-8 in config output: {e}"))
    }

    /// Writes the rapified config to the output.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or if the config cannot be rapified.
    pub fn write_rapified<O: Write>(&self, output: &mut O) -> Result<(), Error> {
        let mut writer = BufWriter::new(output);

        writer.write_all(b"\0raP")?;
        writer.write_all(b"\0\0\0\0\x08\0\0\0")?; // always_0, always_8

        let buffer: Box<[u8]> = vec![0; self.root_body.rapified_length()].into_boxed_slice();
        let mut cursor: Cursor<Box<[u8]>> = Cursor::new(buffer);
        self.root_body
            .write_rapified(&mut cursor, 16)
            .prepend_error("Failed to rapify root class:")?;

        let enum_offset = u32::try_from(16 + cursor.get_ref().len())
            .map_err(|_| error!("config too large: {} bytes", cursor.get_ref().len()))?;
        writer.write_u32::<LittleEndian>(enum_offset)?;

        writer.write_all(cursor.get_ref())?;

        let count = u32::try_from(self.enums.len())
            .map_err(|_| error!("too many enum values: {}", self.enums.len()))?;
        writer.write_u32::<LittleEndian>(count)?;
        for (name, value) in &self.enums {
            writer.write_all(name.as_bytes())?;
            writer.write_all(b"\0")?;
            writer.write_i32::<LittleEndian>(*value)?;
        }
        writer.flush()?;

        Ok(())
    }

    /// Returns the rapified config as a `Cursor`.
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be rapified.
    pub fn to_cursor(&self) -> Result<Cursor<Box<[u8]>>, Error> {
        let len = self.root_body.rapified_length() + 16 + self.rapified_enum_table_length();

        let buffer: Box<[u8]> = vec![0; len].into_boxed_slice();
        let mut cursor: Cursor<Box<[u8]>> = Cursor::new(buffer);
        self.write_rapified(&mut cursor)?;

        Ok(cursor)
    }

    /// Reads the unrapified config from input, preprocessing it.
    ///
    /// `path` is the path to the input if it is known and is used for relative includes and error
    /// messages. `includefolders` are the folders searched recursively for absolute includes.
    ///
    /// # Errors
    ///
    /// Returns an error on preprocessing or parse failures.
    pub fn read<I: Read>(
        input: &mut I,
        path: Option<PathBuf>,
        includefolders: &[PathBuf],
        warnings: &mut dyn WarningSink,
    ) -> Result<Config, Error> {
        Self::read_in(
            input,
            path,
            &IncludeFolders::new(includefolders.to_vec()),
            warnings,
        )
    }

    /// Like [`Config::read`], with the include folders given as an [`IncludeFolders`] (some of them can be checked
    /// directly instead of searched recursively).
    ///
    /// # Errors
    ///
    /// Returns an error on preprocessing or parse failures.
    pub fn read_in<I: Read>(
        input: &mut I,
        path: Option<PathBuf>,
        includefolders: &IncludeFolders,
        warnings: &mut dyn WarningSink,
    ) -> Result<Config, Error> {
        let mut bytes = Vec::new();
        input
            .read_to_end(&mut bytes)
            .prepend_error("Failed to read input file:")?;
        if is_rapified(&bytes) {
            return Config::read_rapified(&mut Cursor::new(bytes));
        }
        let buffer =
            String::from_utf8(bytes).map_err(|e| error!("Failed to read input file: {}", e))?;
        let buffer_ends_a_line = buffer.is_empty() || buffer.ends_with(['\r', '\n']);

        let (preprocessed, info) = preprocess_in(buffer, path, includefolders, warnings)
            .prepend_error("Failed to preprocess config:")?;

        // The preprocessor ends its output with a line end. A file without one has its last entry
        // end at the end of the file, which CfgConvert rejects unless it is closed by `;` or `}`.
        let preprocessed = match preprocessed.strip_suffix('\n') {
            Some(text) if !buffer_ends_a_line => text.to_string(),
            _ => preprocessed,
        };

        let mut found = ConfigWarnings(Vec::new());

        let mut ctx = EvalCtx::default();
        let parsed = config_grammar::config(&preprocessed, &mut found, &mut ctx);
        let result = match ctx.error.take() {
            // the warnings were raised by the fallback rules tried after the failed __EVAL/__EXEC
            Some((pos, e)) => return Err(located_error(&info, &preprocessed, pos, &e)),
            _ => ConfigParseErrorExt::format_error(parsed, &info, &preprocessed),
        };

        // PEG backtracking runs rule actions more than once; report each warning once.
        let mut seen = std::collections::HashSet::new();
        found.0.retain(|w| seen.insert((w.0, w.1.clone())));

        for w in found.0 {
            let location = if warnings.is_suppressed(w.2) {
                (None, None)
            } else {
                let lines_before = preprocessed[..w.0].chars().filter(|c| c == &'\n').count();
                let idx = min(lines_before, info.line_origins.len().saturating_sub(1)); // origins are 1-based line numbers
                let file = info
                    .line_origins
                    .get(idx)
                    .and_then(|o| o.1.as_ref())
                    .map(|p| p.display().to_string());
                let line = match info.line_origins.get(idx) {
                    Some(o) => o.0,
                    None => u32::try_from(lines_before + 1)
                        .map_err(|_| error!("line number out of range"))?,
                };

                (file, Some(line))
            };

            let mut warning = Warning::new(w.2, w.1);
            if let Some(file) = location.0 {
                warning = warning.in_file(file);
            }
            if let Some(line) = location.1 {
                warning = warning.at_line(line);
            }
            warnings.warn(warning);
        }

        result
    }

    /// Preprocesses and parses input string.
    ///
    /// `path` is the path to the input if it is known and is used for relative includes and error
    /// messages. `includefolders` are the folders searched recursively for absolute includes.
    ///
    /// # Errors
    ///
    /// Returns an error on preprocessing or parse failures.
    pub fn from_string(
        input: String,
        path: Option<PathBuf>,
        includefolders: &[PathBuf],
        warnings: &mut dyn WarningSink,
    ) -> Result<Config, Error> {
        let mut cursor = Cursor::new(input.into_bytes());
        Self::read(&mut cursor, path, includefolders, warnings)
    }

    /// Like [`Config::from_string`], with the include folders given as an [`IncludeFolders`].
    ///
    /// # Errors
    ///
    /// Returns an error on preprocessing or parse failures.
    pub fn from_string_in(
        input: String,
        path: Option<PathBuf>,
        includefolders: &IncludeFolders,
        warnings: &mut dyn WarningSink,
    ) -> Result<Config, Error> {
        let mut cursor = Cursor::new(input.into_bytes());
        Self::read_in(&mut cursor, path, includefolders, warnings)
    }

    /// Reads the rapified config from input.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not a rapified config or cannot be parsed.
    pub fn read_rapified<I: Read + Seek>(input: &mut I) -> Result<Config, Error> {
        let mut reader = BufReader::new(input);

        let mut buffer = [0; 4];
        reader.read_exact(&mut buffer)?;

        if &buffer != b"\0raP" {
            return Err(error!("File doesn't seem to be a rapified config."));
        }

        let root_body = ConfigClass::read_rapified(&mut reader, 0, &mut ReadBudget::new())?;
        let enums = read_enum_table(&mut reader);

        Ok(Config { root_body, enums })
    }
}
