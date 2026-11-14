//! The script API index: classes, enums and top-level declarations of one script set (vanilla or a mod).
//!
//! The index is dayzmake's own format. It is serialized as JSON by the cache (see `INDEX_FORMAT`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Version of the index layout. Bump it on every change of the model; the cache treats other values as stale.
pub const INDEX_FORMAT: u32 = 1;

/// Size and modification time of a file an index was built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputStamp {
    /// Path of the input file.
    pub path: PathBuf,
    /// Size in bytes.
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch.
    pub mtime_ns: u128,
}

/// A script module: one PBO that holds `1_Core` .. `5_Mission` directories.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    /// PBO prefix, `\`-separated.
    pub prefix: String,
    /// PBO file relative to its install, e.g. `dta/scripts.pbo` or `addons/x.pbo`.
    pub pbo: String,
}

/// A place in the script sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    /// Index into `ApiIndex::modules`.
    pub module: usize,
    /// Entry name inside the PBO, `\`-separated.
    pub path: String,
    /// 1-based line of the first token of the declaration (an attribute or modifier, not its doc comment).
    pub line: u32,
}

/// A class (or struct) with all its declarations merged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Class {
    /// Class name.
    pub name: String,
    /// Template parameter names, e.g. `["K", "V"]` for `class Pair<Class K, typename V>`.
    pub type_params: Vec<String>,
    /// Base as written in the first declaration that names one, e.g. `Param<K>`; see `Class::base_name`.
    pub base: Option<String>,
    /// Other bases of the same class in `#ifdef` branches that are inactive for the indexed defines.
    pub alt_bases: Vec<String>,
    /// True when every declaration is `modded class`, so the class itself lives in another script set.
    pub modded_only: bool,
    /// First doc comment sentence of the first documented declaration.
    pub doc: Option<String>,
    /// Declarations in load order; `decls[0]` is the main one (an active one if there is one), modded parts and
    /// declarations in inactive branches follow. One entry per file.
    pub decls: Vec<FileRef>,
    /// Members in first-seen order. Declarations that repeat a member (modded overrides) are not listed twice;
    /// the first declaration wins (an active one beats an inactive one), so a later `override` flag is lost.
    pub members: Vec<Member>,
    /// Condition of the innermost inactive `#ifdef` branch the declaration is in, as written (`DIAG_DEVELOPER`,
    /// `!SERVER`); `None` when it is active for the indexed defines.
    pub cond: Option<String>,
}

impl Class {
    /// The base class name without template arguments.
    #[must_use]
    pub fn base_name(&self) -> Option<&str> {
        self.base
            .as_deref()
            .map(|b| b.split('<').next().unwrap_or(b).trim())
    }
}

/// Kind of a class member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberKind {
    /// A method or function.
    Method,
    /// A field, constant or variable.
    Field,
}

/// A member, typedef, function or variable declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    /// Method or field.
    pub kind: MemberKind,
    /// Member name.
    pub name: String,
    /// Normalized declaration without body, doc and attributes: whitespace collapsed, `ref`, `autoptr`, `owned`,
    /// `notnull` and `reference` dropped, `proto` shown as `native`.
    pub signature: String,
    /// Kept modifiers (`static`, `override`, `protected`, `native`, `const`, `sealed`, `abstract`, `final`, ...).
    pub modifiers: Vec<String>,
    /// First doc comment sentence.
    pub doc: Option<String>,
    /// Marked with the `[Obsolete]` attribute.
    pub obsolete: bool,
    /// Where it is declared.
    pub decl: FileRef,
    /// Condition of the innermost inactive `#ifdef` branch the declaration is in, as written (`DIAG_DEVELOPER`,
    /// `!SERVER`); `None` when it is active for the indexed defines.
    pub cond: Option<String>,
}

/// One item of an enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumItem {
    /// Item name.
    pub name: String,
    /// Numeric value, implicit ones counted up from the previous item; `None` for non-numeric expressions.
    pub value: Option<i64>,
    /// First doc comment sentence.
    pub doc: Option<String>,
}

/// An enum with all its declarations (`modded enum` parts) merged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Enum {
    /// Enum name.
    pub name: String,
    /// Underlying type, if declared.
    pub base: Option<String>,
    /// Declarations in load order.
    pub decls: Vec<FileRef>,
    /// Items in declaration order.
    pub items: Vec<EnumItem>,
    /// Condition of the innermost inactive `#ifdef` branch the declaration is in, as written (`DIAG_DEVELOPER`,
    /// `!SERVER`); `None` when it is active for the indexed defines.
    pub cond: Option<String>,
}

/// The index of one script set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiIndex {
    /// Layout version, `INDEX_FORMAT`.
    pub format: u32,
    /// The dayzmake version that built the index; the cache treats other versions as stale.
    #[serde(default)]
    pub version: String,
    /// `vanilla` or the mod name.
    pub name: String,
    /// Human-readable title.
    pub title: String,
    /// Build or version the index was made from.
    pub build: String,
    /// Files the index was built from.
    pub inputs: Vec<InputStamp>,
    /// Script modules, referenced by `FileRef::module`.
    pub modules: Vec<Module>,
    /// Classes by name.
    pub classes: BTreeMap<String, Class>,
    /// Enums by name.
    pub enums: BTreeMap<String, Enum>,
    /// Top-level `typedef`s (`name` is the alias).
    pub typedefs: Vec<Member>,
    /// Top-level functions.
    pub functions: Vec<Member>,
    /// Top-level variables and constants.
    pub globals: Vec<Member>,
    /// Compile-time defines per PBO prefix.
    pub defines: BTreeMap<String, Vec<String>>,
}

impl ApiIndex {
    /// An empty index of the current format.
    #[must_use]
    pub fn new(name: &str, title: &str, build: &str) -> Self {
        ApiIndex {
            format: INDEX_FORMAT,
            version: env!("CARGO_PKG_VERSION").to_string(),
            name: name.to_string(),
            title: title.to_string(),
            build: build.to_string(),
            inputs: Vec::new(),
            modules: Vec::new(),
            classes: BTreeMap::new(),
            enums: BTreeMap::new(),
            typedefs: Vec::new(),
            functions: Vec::new(),
            globals: Vec::new(),
            defines: BTreeMap::new(),
        }
    }
}
