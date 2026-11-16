//! `dayzmake mcp`: a Model Context Protocol server on stdin/stdout.
//!
//! Newline-delimited JSON-RPC 2.0, one request per line, tools only. The tools wrap the library
//! functions behind the CLI commands. Tool calls run one after the other; warnings a call raises
//! are returned in its result, and nothing but JSON-RPC frames is ever written to stdout.
//!
//! The tools take file paths as given (relative to the server's working directory) and act with
//! the permissions of the user running the server.

use std::fs::File;
use std::io::{self, BufRead, Error, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::api::query::Query;
use crate::api::steam::Branch;
use crate::api::{self, ApiContext};
use crate::cli_io::AtomicTarget;
use crate::commands::{binarize, config, extract, p3d, paa, pbo, sign};
use dayzmake_core::error;
use dayzmake_core::error::ErrorExt;
use dayzmake_core::extract::ExtractOptions;
use dayzmake_core::p3d::model_magic;
use dayzmake_core::pbo::UnpackOptions;
use dayzmake_core::preprocess::IncludeFolders;
use dayzmake_core::sign::BISignVersion;
use dayzmake_core::warning::{WarningKind, WarningSink, Warnings};

/// Longest request line accepted; a longer one is answered with an error and skipped.
const MAX_LINE: usize = 16 * 1024 * 1024;
/// Longest tool output returned, in bytes.
const MAX_OUTPUT: usize = 200_000;
/// Default for the `max` argument of the api tools.
const DEFAULT_MAX: usize = 120;
/// Protocol versions the server echoes back; anything else gets the first one.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

type Args = Map<String, Value>;

/// One line read from the request stream.
enum Line {
    Eof,
    Text(Vec<u8>),
    TooLong,
}

/// Reads one line of at most `cap` bytes. A longer line is consumed to its end without being kept.
fn read_line<R: BufRead>(input: &mut R, cap: usize) -> io::Result<Line> {
    let mut buf = Vec::new();
    let mut too_long = false;
    loop {
        let chunk = match input.fill_buf() {
            Ok(chunk) => chunk,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if chunk.is_empty() {
            return Ok(if too_long {
                Line::TooLong
            } else if buf.is_empty() {
                Line::Eof
            } else {
                Line::Text(buf)
            });
        }
        let newline = chunk.iter().position(|&b| b == b'\n');
        let take = newline.unwrap_or(chunk.len());
        if !too_long {
            if buf.len().saturating_add(take) > cap {
                too_long = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(chunk.get(..take).unwrap_or_default());
            }
        }
        input.consume(take + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(if too_long {
                Line::TooLong
            } else {
                Line::Text(buf)
            });
        }
    }
}

/// Serves requests from `input` until it ends, writing one reply line per request to `output`.
///
/// # Errors
///
/// Returns an error when reading the input or writing the output fails.
pub fn serve<R: BufRead, W: Write>(input: R, output: W, verbose: bool) -> Result<(), Error> {
    serve_with_limit(input, output, MAX_LINE, verbose)
}

fn serve_with_limit<R: BufRead, W: Write>(
    mut input: R,
    mut output: W,
    cap: usize,
    verbose: bool,
) -> Result<(), Error> {
    loop {
        let reply = match read_line(&mut input, cap)? {
            Line::Eof => return Ok(()),
            Line::TooLong => Some(error_reply(
                &Value::Null,
                INVALID_REQUEST,
                "request line too long",
            )),
            Line::Text(line) => handle_line(&line, verbose),
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
}

fn error_reply(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Handles one request line; `None` for notifications and blank lines.
fn handle_line(line: &[u8], verbose: bool) -> Option<Value> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return Some(error_reply(&Value::Null, PARSE_ERROR, "parse error"));
    };
    let Value::Object(request) = value else {
        return Some(error_reply(
            &Value::Null,
            INVALID_REQUEST,
            "a request must be a JSON object",
        ));
    };
    let id = request.get("id");
    if id.is_some_and(|id| !id.is_string() && !id.is_number()) {
        return Some(error_reply(&Value::Null, INVALID_REQUEST, "invalid id"));
    }
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        if request.contains_key("result") || request.contains_key("error") {
            return None; // a response to a request we never send
        }
        return Some(error_reply(
            id.unwrap_or(&Value::Null),
            INVALID_REQUEST,
            "missing method",
        ));
    };
    // notifications never get a reply
    let id = id?;
    Some(match dispatch(method, request.get("params"), verbose) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => error_reply(id, code, &message),
    })
}

fn dispatch(method: &str, params: Option<&Value>, verbose: bool) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let asked = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str);
            let version = VERSIONS
                .iter()
                .find(|v| Some(**v) == asked)
                .unwrap_or(&VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "dayzmake", "version": env!("CARGO_PKG_VERSION")},
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let params = params
                .and_then(Value::as_object)
                .ok_or_else(|| (INVALID_PARAMS, "params must be an object".to_string()))?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| (INVALID_PARAMS, "params.name must be a string".to_string()))?;
            let empty = Map::new();
            let args = match params.get("arguments") {
                None | Some(Value::Null) => &empty,
                Some(Value::Object(m)) => m,
                Some(_) => {
                    return Err((
                        INVALID_PARAMS,
                        "params.arguments must be an object".to_string(),
                    ));
                }
            };
            if !tools().as_array().is_some_and(|t| {
                t.iter()
                    .any(|t| t.get("name").and_then(Value::as_str) == Some(name))
            }) {
                return Err((INVALID_PARAMS, format!("unknown tool \"{name}\"")));
            }
            Ok(run_tool(name, args, verbose))
        }
        _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
    }
}

/// Runs a tool and wraps its output (or its failure) as an MCP tool result.
fn run_tool(name: &str, args: &Args, verbose: bool) -> Value {
    let (offset, length) = match window_args(name, args) {
        Ok(w) => w,
        Err(e) => {
            return json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true});
        }
    };
    // A panic in a tool must not kill the server. The closure only borrows plain data and it
    // holds no global state (a Binarizer is dropped while unwinding). Every call has its own
    // warnings, with their own quotas.
    let mut sink = Warnings::new();
    if verbose {
        sink.unlimited();
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| call_tool(name, args, &mut sink)));
    // notes (Proton progress, binarize.exe output) go to the server's stderr as they always did;
    // stdout carries only JSON-RPC frames
    let mut warnings: Vec<String> = Vec::new();
    for w in sink.warnings() {
        if w.kind == WarningKind::Note {
            // a failed write to stderr (the reader is gone) cannot be reported anywhere, and must not
            // stop the server: `eprintln!` would panic on it
            let _ = writeln!(std::io::stderr(), "{w}");
        } else {
            warnings.push(w.to_string());
        }
    }
    let (text, is_error) = match outcome {
        Ok(Ok(out)) => (
            with_warnings(&window(&out, offset, length), &warnings),
            false,
        ),
        Ok(Err(e)) => (
            with_warnings(&window(&e.to_string(), 0, MAX_OUTPUT), &warnings),
            true,
        ),
        Err(_) => (
            format!("internal error: the tool \"{name}\" panicked"),
            true,
        ),
    };
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

/// The captured warnings come first, so cutting the output never drops them.
fn with_warnings(text: &str, warnings: &[String]) -> String {
    if warnings.is_empty() {
        return text.to_string();
    }
    format!("warnings:\n{}\n\n{text}", warnings.join("\n"))
}

/// The byte window (`offset`, `length`) the call asks for. Only the tools that return whole files
/// (`api_source` with `cat`, `pbo_cat`) take `offset` and `length`; the others start at 0.
fn window_args(name: &str, args: &Args) -> Result<(usize, usize), Error> {
    if !matches!(name, "api_source" | "pbo_cat") {
        return Ok((0, MAX_OUTPUT));
    }
    let number = |key: &str, default: usize| -> Result<usize, Error> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| error!("argument \"{key}\" must be a non-negative integer")),
        }
    };
    Ok((
        number("offset", 0)?,
        number("length", MAX_OUTPUT)?.min(MAX_OUTPUT),
    ))
}

/// At most `length` bytes of `text` from byte `offset` on, cut on character boundaries (the start moves
/// forward, the end back). Says what was left out and which `offset` fetches the rest.
fn window(text: &str, offset: usize, length: usize) -> String {
    let total = text.len();
    if offset == 0 && total <= length {
        return text.to_string();
    }
    if offset >= total {
        return format!("[offset {offset} is past the end of the {total} bytes]");
    }
    let mut start = offset;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = start.saturating_add(length).min(total);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let body = text.get(start..end).unwrap_or_default();
    if end < total {
        format!(
            "{body}\n[truncated: showing bytes {start}-{end} of {total}; call again with offset={end}]"
        )
    } else {
        format!("{body}\n[showing bytes {start}-{end} of {total}]")
    }
}

// ---- tool definitions ----

/// Builds one tool entry. Property kinds: `s` string, `b` boolean, `i` integer, `a` array of strings.
fn tool(name: &str, description: &str, props: &[(&str, char, &str)], required: &[&str]) -> Value {
    let mut properties = Map::new();
    for (key, kind, doc) in props {
        let mut schema = match kind {
            'b' => json!({"type": "boolean"}),
            'i' => json!({"type": "integer", "minimum": 0}),
            'a' => json!({"type": "array", "items": {"type": "string"}}),
            _ => json!({"type": "string"}),
        };
        if let Some(o) = schema.as_object_mut() {
            o.insert("description".to_string(), json!(doc));
        }
        properties.insert((*key).to_string(), schema);
    }
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
    })
}

const API_COMMON: [(&str, char, &str); 4] = [
    (
        "mods",
        's',
        "Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried.",
    ),
    (
        "experimental",
        'b',
        "Use the experimental branch instead of stable.",
    ),
    (
        "dayz",
        's',
        "DayZ client/server directory (uses <dir>/dta/scripts.pbo) instead of the Steam install.",
    ),
    ("refresh", 'b', "Rebuild the cached index first."),
];
const JSON_ARG: (&str, char, &str) = ("json", 'b', "Return JSON instead of text.");
const MAX_ARG: (&str, char, &str) = (
    "max",
    'i',
    "At most this many lines or items (default 120).",
);
const FORCE_ARG: (&str, char, &str) = ("force", 'b', "Overwrite existing target files.");
const OFFSET_ARG: (&str, char, &str) = (
    "offset",
    'i',
    "Start at this byte of the output (default 0). A cut-off reply names the offset that fetches the rest.",
);
const LENGTH_ARG: (&str, char, &str) = (
    "length",
    'i',
    "Return at most this many bytes (default and maximum 200000).",
);

/// `api_grep`: the common api arguments except `refresh` (grep reads the PBOs and builds no index).
fn grep_tool() -> Value {
    let mut props: Vec<(&str, char, &str)> = vec![
        (
            "regex",
            's',
            "Regular expression, matched against each line.",
        ),
        ("ignore_case", 'b', "Match case-insensitively."),
    ];
    props.extend(API_COMMON.iter().filter(|p| p.0 != "refresh"));
    props.push(JSON_ARG);
    props.push(MAX_ARG);
    tool(
        "api_grep",
        "Full-text search over the script bodies (every .c file of vanilla and the selected mods, read from the PBOs). Matches raw lines, comments included.",
        &props,
        &["regex"],
    )
}

fn api_tool(
    name: &str,
    description: &str,
    own: &[(&str, char, &str)],
    required: &[&str],
    json_and_max: (bool, bool),
) -> Value {
    let mut props: Vec<(&str, char, &str)> = own.to_vec();
    props.extend(API_COMMON);
    if json_and_max.0 {
        props.push(JSON_ARG);
    }
    if json_and_max.1 {
        props.push(MAX_ARG);
    }
    tool(name, description, &props, required)
}

#[expect(
    clippy::too_many_lines,
    reason = "one static table entry per tool keeps the schemas in one place"
)]
fn tools() -> Value {
    let key_arg = (
        "key",
        's',
        "Private key (.biprivatekey): sign the PBO after packing.",
    );
    let pack_props = [
        ("source", 's', "Folder to pack."),
        ("target", 's', "PBO file to write."),
        ("exclude", 'a', "Glob patterns to leave out."),
        ("headerext", 'a', "Header extensions as key=value."),
        key_arg,
        FORCE_ARG,
    ];
    let mut build_props = pack_props.to_vec();
    build_props.push((
        "include",
        'a',
        "Folders to search for includes (the current directory is only checked directly, not searched).",
    ));
    build_props.push((
        "proton_binarize",
        'b',
        "Linux: binarize models with DayZ Tools' binarize.exe under Proton.",
    ));
    json!([
        api_tool(
            "api_class",
            "Members of a DayZ script class from the installed scripts.pbo (vanilla; mods only when selected with mods).",
            &[
                ("name", 's', "Class name."),
                ("inherited", 'b', "Also show the base classes."),
                (
                    "mods_only",
                    'b',
                    "Show only what the selected mods declare."
                ),
            ],
            &["name"],
            (true, true)
        ),
        api_tool(
            "api_find",
            "Case-insensitive regex over member names, signatures, docs, class names and enum items (so ^OnRPC finds the OnRPC methods).",
            &[
                ("regex", 's', "The regular expression."),
                (
                    "class",
                    's',
                    "Search only the members of this class or enum."
                ),
            ],
            &["regex"],
            (true, true)
        ),
        api_tool(
            "api_subclasses",
            "Subclasses of a class.",
            &[
                ("name", 's', "Class name."),
                ("all", 'b', "Include indirect subclasses."),
            ],
            &["name"],
            (true, true)
        ),
        api_tool(
            "api_enum",
            "Items of an enum.",
            &[("name", 's', "Enum name.")],
            &["name"],
            (true, true)
        ),
        api_tool(
            "api_chain",
            "Base class chain of a class.",
            &[("name", 's', "Class name.")],
            &["name"],
            (true, false)
        ),
        api_tool(
            "api_source",
            "Files and lines that declare a class or enum; with cat the full source files, with context or lines part of them.",
            &[
                ("name", 's', "Class or enum name."),
                (
                    "cat",
                    'b',
                    "Return the full source files (with json: the files as JSON)."
                ),
                (
                    "context",
                    'i',
                    "Return n lines around each declaration, with line numbers (excludes cat and lines)."
                ),
                (
                    "lines",
                    's',
                    "Return lines a to b of each file as \"a-b\", with line numbers (excludes cat and context)."
                ),
                OFFSET_ARG,
                LENGTH_ARG,
                (
                    "target",
                    's',
                    "With cat: write the files to this file instead of returning them."
                ),
                FORCE_ARG,
            ],
            &["name"],
            (true, true)
        ),
        grep_tool(),
        api_tool(
            "api_defines",
            "Compile-time defines of the selected indexes, optionally filtered by a regex.",
            &[("regex", 's', "Filter regular expression.")],
            &[],
            (true, true)
        ),
        api_tool(
            "api_update",
            "Rebuild the cached indexes now. With mods=all this indexes every installed mod, which warms the caches for later queries.",
            &[],
            &[],
            (false, false)
        ),
        tool(
            "api_mods",
            "List the installed mods (workshop id, name, directory, whether they have scripts); the names and ids can be used in the mods argument of the other api tools. Reading them is quick; querying all of them with mods=all builds the caches on the first run and can take a while; api_update with mods=all warms them.",
            &[
                (
                    "experimental",
                    'b',
                    "Use the experimental branch instead of stable."
                ),
                (
                    "dayz",
                    's',
                    "DayZ client/server directory instead of the Steam install.",
                ),
                JSON_ARG,
            ],
            &[]
        ),
        tool(
            "pbo_inspect",
            "List the entries of a PBO, or describe a model (P3D: format, LODs, textures, selections).",
            &[
                ("path", 's', "PBO or P3D file."),
                (
                    "json",
                    'b',
                    "PBOs only: return the header as JSON with the raw bytes of names as hex."
                ),
            ],
            &["path"],
        ),
        tool(
            "pbo_cat",
            "Read one entry of a PBO: returned as text if it is UTF-8, otherwise write it to target.",
            &[
                ("pbo", 's', "PBO file."),
                ("file", 's', "Entry name."),
                (
                    "target",
                    's',
                    "Write the entry to this file instead of returning it (required for binary data)."
                ),
                FORCE_ARG,
                OFFSET_ARG,
                LENGTH_ARG,
            ],
            &["pbo", "file"],
        ),
        tool(
            "pbo_unpack",
            "Unpack a PBO into a folder, all of it or the entries selected by only/exclude. Unsafe entry names are skipped with a warning.",
            &[
                ("pbo", 's', "PBO file."),
                ("target", 's', "Folder to extract into."),
                FORCE_ARG,
                ("use_prefix", 'b', "Extract into <target>/<prefix>/."),
                (
                    "exclude",
                    'a',
                    "Glob patterns of entries to leave out (case-insensitive, / separators)."
                ),
                (
                    "only",
                    'a',
                    "Glob patterns: write only the entries that match one (exclude wins)."
                ),
            ],
            &["pbo", "target"],
        ),
        tool(
            "extract",
            "Unpack every PBO of a DayZ install, a mod folder or a single PBO into one tree <target>/<prefix>/<entry>, optionally derapified, and write the prefix -> PBO map pbo-map.tsv. The tree replaces an existing target atomically with force.",
            &[
                (
                    "source",
                    's',
                    "DayZ client or server folder, mod folder (dta/ and addons/ are searched) or one PBO."
                ),
                ("target", 's', "Folder to write the tree to."),
                FORCE_ARG,
                (
                    "derapify",
                    'b',
                    "Write every rapified .bin as a .cpp of the same name."
                ),
                (
                    "exclude",
                    'a',
                    "Glob patterns of entries to leave out (case-insensitive, / separators)."
                ),
                (
                    "only",
                    'a',
                    "Glob patterns: write only the entries that match one (exclude wins)."
                ),
            ],
            &["source", "target"],
        ),
        tool(
            "pbo_pack",
            "Pack a folder into a PBO without binarizing or rapifying.",
            &pack_props,
            &["source", "target"]
        ),
        tool(
            "pbo_build",
            "Build a PBO from a folder, rapifying configs and binarizing models.",
            &build_props,
            &["source", "target"]
        ),
        tool(
            "config_rapify",
            "Preprocess and rapify a config file.",
            &[
                ("source", 's', "Config source file."),
                ("target", 's', "File to write."),
                ("include", 'a', "Folders to search for includes."),
                FORCE_ARG,
            ],
            &["source", "target"],
        ),
        tool(
            "config_derapify",
            "Convert a rapified config back to text.",
            &[
                ("source", 's', "Rapified config."),
                ("target", 's', "Write the text here; absent: return it."),
                (
                    "indent",
                    's',
                    "Indentation per level: spaces and tabs only, at most 64 characters, default four spaces.",
                ),
                FORCE_ARG,
            ],
            &["source"],
        ),
        tool(
            "config_preprocess",
            "Preprocess a file (macros, includes).",
            &[
                ("source", 's', "File to preprocess."),
                ("target", 's', "Write the result here; absent: return it."),
                ("include", 'a', "Folders to search for includes."),
                FORCE_ARG,
            ],
            &["source"],
        ),
        tool(
            "paa_to_image",
            "Convert a PAA texture to an image (the format follows the target extension, e.g. png).",
            &[
                ("source", 's', "PAA file."),
                ("target", 's', "Image file to write."),
                FORCE_ARG
            ],
            &["source", "target"],
        ),
        tool(
            "image_to_paa",
            "Convert an image to a PAA texture.",
            &[
                ("source", 's', "Image file."),
                ("target", 's', "PAA file to write."),
                ("type", 's', "PAA type, e.g. dxt1, dxt5 (default dxt5)."),
                (
                    "compress",
                    'b',
                    "LZO-compress DXT mipmaps wider than 128 px (always done; kept for compatibility)."
                ),
                FORCE_ARG,
            ],
            &["source", "target"],
        ),
        tool(
            "key_generate",
            "Generate a key pair (<name>.biprivatekey and <name>.bikey).",
            &[
                ("name", 's', "Path of the key without extension."),
                FORCE_ARG
            ],
            &["name"],
        ),
        tool(
            "pbo_sign",
            "Sign a PBO with a private key.",
            &[
                ("private_key", 's', "Private key file."),
                ("pbo", 's', "PBO file."),
                (
                    "signature",
                    's',
                    "Signature file (default <pbo>.<key>.bisign)."
                ),
                ("v2", 'b', "Write an older v2 signature."),
                FORCE_ARG,
            ],
            &["private_key", "pbo"],
        ),
        tool(
            "pbo_verify",
            "Verify the signature of a PBO with a public key.",
            &[
                ("public_key", 's', "Public key file (.bikey)."),
                ("pbo", 's', "PBO file."),
                (
                    "signature",
                    's',
                    "Signature file (default <pbo>.<key>.bisign)."
                ),
            ],
            &["public_key", "pbo"],
        ),
        tool(
            "binarize",
            "Binarize a model with BI's binarize.exe (Windows; on Linux through Proton).",
            &[
                ("source", 's', "Model file."),
                ("target", 's', "File to write."),
                FORCE_ARG
            ],
            &["source", "target"],
        ),
    ])
}

// ---- arguments ----

fn opt_str(args: &Args, key: &str) -> Result<Option<String>, Error> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(error!("argument \"{key}\" must be a string")),
    }
}

fn str_arg(args: &Args, key: &str) -> Result<String, Error> {
    opt_str(args, key)?.ok_or_else(|| error!("missing required argument \"{key}\""))
}

fn path_arg(args: &Args, key: &str) -> Result<PathBuf, Error> {
    str_arg(args, key).map(PathBuf::from)
}

fn opt_path(args: &Args, key: &str) -> Result<Option<PathBuf>, Error> {
    Ok(opt_str(args, key)?.map(PathBuf::from))
}

fn bool_arg(args: &Args, key: &str) -> Result<bool, Error> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(error!("argument \"{key}\" must be a boolean")),
    }
}

fn strs_arg(args: &Args, key: &str) -> Result<Vec<String>, Error> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| error!("argument \"{key}\" must be an array of strings"))
            })
            .collect(),
        Some(_) => Err(error!("argument \"{key}\" must be an array of strings")),
    }
}

fn max_arg(args: &Args) -> Result<usize, Error> {
    match args.get("max") {
        None | Some(Value::Null) => Ok(DEFAULT_MAX),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| error!("argument \"max\" must be a non-negative integer")),
    }
}

/// The include folders of a call: the given ones, then the current directory like the CLI (only checked directly).
fn include_folders(args: &Args) -> Result<IncludeFolders, Error> {
    let folders: Vec<PathBuf> = strs_arg(args, "include")?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    Ok(IncludeFolders::new(folders).with_direct(vec![PathBuf::from(".")]))
}

// ---- tools ----

/// Runs the named tool and returns its output text.
fn call_tool(name: &str, args: &Args, warnings: &mut dyn WarningSink) -> Result<String, Error> {
    match name {
        n if n.starts_with("api_") => api_tool_call(n, args, warnings),
        "pbo_inspect" => pbo_inspect(&path_arg(args, "path")?, bool_arg(args, "json")?),
        "pbo_cat" => pbo_cat(args),
        "pbo_unpack" => {
            let (pbo_path, target) = (path_arg(args, "pbo")?, path_arg(args, "target")?);
            let mut options = UnpackOptions::default();
            options.force = bool_arg(args, "force")?;
            options.use_prefix = bool_arg(args, "use_prefix")?;
            options.exclude = strs_arg(args, "exclude")?;
            options.only = strs_arg(args, "only")?;
            pbo::cmd_unpack(&mut open(&pbo_path)?, &target, &options, warnings)?;
            Ok(format!(
                "unpacked {} to {}",
                pbo_path.display(),
                target.display()
            ))
        }
        "extract" => {
            let (source, target) = (path_arg(args, "source")?, path_arg(args, "target")?);
            let mut options = ExtractOptions::default();
            options.force = bool_arg(args, "force")?;
            options.derapify = bool_arg(args, "derapify")?;
            options.exclude = strs_arg(args, "exclude")?;
            options.only = strs_arg(args, "only")?;
            let summary = extract::cmd_extract(&source, &target, &options, warnings)?;
            Ok(format!(
                "extracted {} files from {} PBOs to {}",
                summary.files(),
                summary.pbos(),
                target.display()
            ))
        }
        "pbo_pack" | "pbo_build" => pack(name == "pbo_build", args, warnings),
        _ => file_tool(name, args, warnings),
    }
}

/// The tools for configs, textures, keys, signatures and models.
fn file_tool(name: &str, args: &Args, warnings: &mut dyn WarningSink) -> Result<String, Error> {
    match name {
        "config_rapify" => {
            let source = path_arg(args, "source")?;
            let mut out = Vec::new();
            config::cmd_rapify(
                &mut open(&source)?,
                &mut out,
                Some(source),
                &include_folders(args)?,
                warnings,
            )?;
            write_target(&path_arg(args, "target")?, &out, bool_arg(args, "force")?)
        }
        "config_derapify" => {
            let mut out = Vec::new();
            let indent = opt_str(args, "indent")?;
            config::cmd_derapify(
                &mut open(&path_arg(args, "source")?)?,
                &mut out,
                indent.as_deref(),
            )?;
            text_or_target(args, &out)
        }
        "config_preprocess" => {
            let source = path_arg(args, "source")?;
            let mut out = Vec::new();
            config::cmd_preprocess(
                &mut open(&source)?,
                &mut out,
                Some(source),
                &include_folders(args)?,
                warnings,
            )?;
            text_or_target(args, &out)
        }
        "paa_to_image" => {
            let target = path_arg(args, "target")?;
            paa::cmd_paa2img(
                &path_arg(args, "source")?,
                &target,
                bool_arg(args, "force")?,
            )?;
            Ok(format!("wrote {}", target.display()))
        }
        "image_to_paa" => {
            let target = path_arg(args, "target")?;
            paa::cmd_img2paa(
                &path_arg(args, "source")?,
                &target,
                &opt_str(args, "type")?.unwrap_or_else(|| "dxt5".to_string()),
                bool_arg(args, "compress")?,
                bool_arg(args, "force")?,
            )?;
            Ok(format!("wrote {}", target.display()))
        }
        "key_generate" => {
            let key = path_arg(args, "name")?;
            sign::cmd_keygen(key.clone(), bool_arg(args, "force")?)?;
            Ok(format!(
                "generated {0}.biprivatekey and {0}.bikey",
                key.display()
            ))
        }
        "pbo_sign" => {
            let version = if bool_arg(args, "v2")? {
                BISignVersion::V2
            } else {
                BISignVersion::V3
            };
            sign::cmd_sign(
                path_arg(args, "private_key")?,
                path_arg(args, "pbo")?,
                opt_path(args, "signature")?,
                version,
                bool_arg(args, "force")?,
            )?;
            Ok("signed".to_string())
        }
        "pbo_verify" => {
            sign::cmd_verify(
                path_arg(args, "public_key")?,
                path_arg(args, "pbo")?,
                opt_path(args, "signature")?,
            )?;
            Ok("signature is valid".to_string())
        }
        "binarize" => {
            let target = path_arg(args, "target")?;
            binarize::cmd_binarize(
                path_arg(args, "source")?,
                &target,
                bool_arg(args, "force")?,
                warnings,
            )?;
            Ok(format!("wrote {}", target.display()))
        }
        _ => Err(error!("unknown tool \"{name}\"")),
    }
}

fn open(path: &Path) -> Result<File, Error> {
    File::open(path).map_err(|e| error!("Failed to open \"{}\": {e}", path.display()))
}

/// Writes `data` to `path` (not over an existing file unless `force`) and says so.
fn write_target(path: &Path, data: &[u8], force: bool) -> Result<String, Error> {
    let mut target = AtomicTarget::create(path, force)?;
    target
        .write_all(data)
        .prepend_error("Failed to write output:")?;
    target.commit()?;
    Ok(format!("wrote {} ({} bytes)", path.display(), data.len()))
}

/// With a `target` argument writes the data there, otherwise returns it as text.
fn text_or_target(args: &Args, data: &[u8]) -> Result<String, Error> {
    match opt_path(args, "target")? {
        Some(target) => write_target(&target, data, bool_arg(args, "force")?),
        None => Ok(String::from_utf8_lossy(data).into_owned()),
    }
}

fn api_context(args: &Args, refresh: bool) -> Result<ApiContext, Error> {
    Ok(ApiContext {
        branch: if bool_arg(args, "experimental")? {
            Branch::Experimental
        } else {
            Branch::Stable
        },
        dayz: opt_path(args, "dayz")?,
        mods: opt_str(args, "mods")?.unwrap_or_default(),
        refresh: refresh || bool_arg(args, "refresh")?,
    })
}

fn api_tool_call(name: &str, args: &Args, warnings: &mut dyn WarningSink) -> Result<String, Error> {
    if name == "api_mods" {
        return api::mods_text(
            &api_context(args, false)?,
            bool_arg(args, "json")?,
            warnings,
        );
    }
    if name == "api_update" {
        return Ok(api::update_lines(&api_context(args, true)?, warnings)?.join("\n"));
    }
    let ctx = api_context(args, false)?;
    let json = bool_arg(args, "json")?;
    if name == "api_grep" {
        return Ok(api::grep_lines(
            &ctx,
            &str_arg(args, "regex")?,
            bool_arg(args, "ignore_case")?,
            max_arg(args)?,
            json,
            warnings,
        )?
        .join("\n"));
    }
    let query = match name {
        "api_class" => Query::Class {
            name: str_arg(args, "name")?,
            inherited: bool_arg(args, "inherited")?,
            mods_only: bool_arg(args, "mods_only")?,
        },
        "api_find" => Query::Find {
            regex: str_arg(args, "regex")?,
            class: opt_str(args, "class")?,
        },
        "api_subclasses" => Query::Sub {
            name: str_arg(args, "name")?,
            all: bool_arg(args, "all")?,
        },
        "api_enum" => Query::Enum {
            name: str_arg(args, "name")?,
        },
        "api_chain" => Query::Chain {
            name: str_arg(args, "name")?,
        },
        "api_source" => {
            let name = str_arg(args, "name")?;
            let context = match args.get("context") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    v.as_u64()
                        .ok_or_else(|| {
                            error!("argument \"context\" must be a non-negative integer")
                        })?
                        .to_string(),
                ),
            };
            let cat = bool_arg(args, "cat")?;
            if let Some(range) =
                api::SrcRange::parse(cat, context.as_deref(), opt_str(args, "lines")?.as_deref())?
            {
                return api::src_range(&ctx, &name, range, json, warnings);
            }
            if cat {
                let text = api::src_cat(&ctx, &name, json, warnings)?;
                return match opt_path(args, "target")? {
                    Some(target) => {
                        write_target(&target, text.as_bytes(), bool_arg(args, "force")?)
                    }
                    None => Ok(text),
                };
            }
            Query::Src { name }
        }
        "api_defines" => Query::Defines {
            regex: opt_str(args, "regex")?,
        },
        _ => return Err(error!("unknown tool \"{name}\"")),
    };
    Ok(api::answer(&ctx, &query, max_arg(args)?, json, false, warnings)?.join("\n"))
}

fn pbo_inspect(path: &Path, json: bool) -> Result<String, Error> {
    let mut file = open(path)?;
    let mut out = Vec::new();
    if model_magic(&mut file)?.is_some() {
        if json {
            return Err(error!("--json is only supported for PBOs."));
        }
        p3d::cmd_inspect(&mut file, &mut out)?;
    } else if json {
        pbo::cmd_inspect_json(&mut file, &mut out)?;
    } else {
        pbo::cmd_inspect(&mut file, &mut out)?;
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn pbo_cat(args: &Args) -> Result<String, Error> {
    let mut out = Vec::new();
    pbo::cmd_cat(
        &mut open(&path_arg(args, "pbo")?)?,
        &mut out,
        &str_arg(args, "file")?,
    )?;
    if let Some(target) = opt_path(args, "target")? {
        return write_target(&target, &out, bool_arg(args, "force")?);
    }
    String::from_utf8(out).map_err(|e| {
        error!(
            "the entry is not UTF-8 text ({} bytes); pass target to write it to a file",
            e.as_bytes().len()
        )
    })
}

/// `pbo_pack` and `pbo_build`: like the CLI commands, including signing with `key`.
fn pack(build: bool, args: &Args, warnings: &mut dyn WarningSink) -> Result<String, Error> {
    let source = path_arg(args, "source")?;
    let target = path_arg(args, "target")?;
    let force = bool_arg(args, "force")?;
    let headerext = strs_arg(args, "headerext")?;
    let exclude = strs_arg(args, "exclude")?;
    let key = opt_path(args, "key")?;
    if build {
        // Only the build can binarize; the binarizer (and its sandbox) lives for this call only.
        let mut binarizer = crate::run::platform_binarizer(bool_arg(args, "proton_binarize")?)?;
        let mut file = AtomicTarget::create(&target, force)?;
        let own = file.own_paths();
        let result = pbo::cmd_build(
            &source,
            &mut file,
            &headerext,
            &exclude,
            &include_folders(args)?,
            &own,
            false, // MCP never writes a PBO to stdout
            binarizer.as_mut(),
            warnings,
        );
        // the build error wins over a cleanup error
        let closed = binarizer.map_or(Ok(()), |b| b.close(warnings));
        result?;
        closed?;
        file.commit()?;
    } else {
        let mut file = AtomicTarget::create(&target, force)?;
        let own = file.own_paths();
        pbo::cmd_pack(
            &source, &mut file, &headerext, &exclude, &own, false, warnings,
        )?;
        file.commit()?;
    }
    if let Some(key) = key {
        sign::cmd_sign(key, target.clone(), None, BISignVersion::V3, force)?;
        return Ok(format!("wrote and signed {}", target.display()));
    }
    Ok(format!("wrote {}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn long_lines_are_skipped_up_to_their_newline() {
        let mut input = Cursor::new(b"0123456789\nabc\nlast".to_vec());
        assert!(matches!(read_line(&mut input, 5), Ok(Line::TooLong)));
        assert!(matches!(read_line(&mut input, 5), Ok(Line::Text(t)) if t == b"abc"));
        assert!(matches!(read_line(&mut input, 5), Ok(Line::Text(t)) if t == b"last"));
        assert!(matches!(read_line(&mut input, 5), Ok(Line::Eof)));
    }

    #[test]
    fn a_line_of_exactly_the_cap_is_accepted() {
        let mut input = Cursor::new(b"12345\n".to_vec());
        assert!(matches!(read_line(&mut input, 5), Ok(Line::Text(t)) if t == b"12345"));
    }

    #[test]
    fn truncation_keeps_characters_whole() {
        let text = "é".repeat(MAX_OUTPUT);
        let cut = window(&text, 0, MAX_OUTPUT);
        assert!(
            cut.contains(
                "[truncated: showing bytes 0-200000 of 400000; call again with offset=200000]"
            ),
            "{}",
            &cut[cut.len() - 90..]
        );
        // an odd length cuts back to a boundary and the next offset follows from it
        let cut = window(&text, 0, 5);
        assert!(
            cut.starts_with("éé\n") && cut.contains("offset=4]"),
            "{cut}"
        );
    }

    #[test]
    fn chunks_reassemble_the_text() {
        let text = "aé\n".repeat(1000);
        let mut out = String::new();
        let mut offset = 0;
        while offset < text.len() {
            let w = window(&text, offset, 777);
            let (body, note) = w.rsplit_once('\n').expect("a note line");
            out.push_str(body);
            offset = note
                .split("offset=")
                .nth(1)
                .and_then(|n| n.trim_end_matches(']').parse().ok())
                .unwrap_or(text.len());
        }
        assert_eq!(out, text);
    }

    #[test]
    fn warnings_come_before_the_cut() {
        let warnings = vec!["w1 [x]".to_string()];
        let out = with_warnings(&window(&"a".repeat(500), 0, 10), &warnings);
        assert!(out.starts_with("warnings:\nw1 [x]\n\n"), "{out}");
        assert!(out.contains("[truncated:"), "{out}");
    }
}
