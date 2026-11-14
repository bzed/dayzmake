# Untrusted-input hardening, script API index and MCP server: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** (1) Make every code path that reads foreign data (PBOs, rapified configs, PAA, P3D, keys, signatures) safe:
no writes outside the target folder, no panics, no unbounded allocation or recursion. (2) Add `dayzmake api`, a
Rust port of the dayz-dev plugin's `build-api-index.py` / `dayz-api.py`, which builds the script API index straight
from the installed game/mod PBOs, caches it and rebuilds it when the PBOs change; stable by default, `--experimental`
for the experimental branch; `api src --cat` prints the full `.c` file from the PBO. (3) Add `dayzmake mcp`, a stdio
MCP server exposing the API queries and the PBO/config/texture/key commands as tools.

**Architecture:** Task 1 fixes `src/pbo.rs` unpack and bounds the binary readers, then a dedicated audit task reviews
all untrusted-input code. Task 2 adds a `src/api/` module (steam discovery, source tree from PBOs, index builder,
query engine, cache) whose query functions return `Vec<String>` so the CLI and MCP share them. Task 3 first makes the
printing commands write to a caller-supplied `Write`, then adds `src/mcp.rs`, a hand-rolled JSON-RPC 2.0 / MCP loop
on stdin/stdout using `serde_json`.

**Tech Stack:** Rust 2024 (toolchain pinned in `rust-toolchain.toml`), docopt, regex, openssl (SHA-1), serde +
serde_json (new), tempfile (tests).

**Spec:** the user's request in this session (no separate spec file):
- task 1: "extracting pbo can write to arbitrary paths if a manipulated pbo has .. in their prefix (maybe there are
  other issues). fix that and do a security review for all code that deals with unknown data."
- task 2: "`~/workspace/skills/dayz-dev-plugin/` has an api index generation script and an api query tool. Add
  support for these things to dayzmake, distinguish between experimental and stable (defaulting to stable). create
  cached api indexes on the fly, keep them up to date automatically. Support retrieving the full c file from the pbo
  on request."
- task 3: "add a mcp server that is able to handle all our functions where it makes sense."

Reference implementation for task 2 (read it, do not modify it):
`/home/bzed/workspace/skills/dayz-dev-plugin/scripts/build-api-index.py`,
`/home/bzed/workspace/skills/dayz-dev-plugin/scripts/dayz-api.py`,
`/home/bzed/workspace/skills/dayz-dev-plugin/scripts/find-dayzserver.sh`,
`/home/bzed/workspace/skills/dayz-dev-plugin/scripts/extract-dayz-sources.sh`.

## Global Constraints

- Work on branch `feature/untrusted-api-mcp` (create it from `main` before Task 1). Commit after every task; never push.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (subagents: use your own model name).
- `make check` must pass at the end of every task (fmt, clippy `-D warnings` with the crate's pedantic lint set, docs,
  cargo-deny, tests, line coverage >= 70). At minimum run `make fmt lint test` per task.
- No `unwrap`/`expect`/`panic!`/indexing in non-test code (crate lints); use `#[expect(..., reason = "...")]` only
  where unavoidable, never `#[allow]`.
- New runtime dependency allowed: `serde_json = "1"` only. No tokio, no MCP SDK crate, no xz/lzma crate.
- Temporary/scratch dirs only under `/tmp` or `/var/tmp` (`mktemp -d /tmp/x.XXXXXX`), never under `$HOME`. Tests
  use `tempfile` (the Makefile exports `TMPDIR=/tmp`).
- Never write into the real Steam/DayZ install. Tests never depend on a real DayZ install; they use fixtures and
  fake Steam roots. Real-install checks are manual smoke steps only.
- Untrusted data = any PBO, rapified config, PAA, P3D, `.bikey`/`.biprivatekey`/`.bisign`, Steam `.vdf`/`.acf`, and
  any file name, prefix or header extension inside them.

## Review Focus

1. A PBO entry or prefix containing `..`, a leading `\`/`/`, a drive (`C:`), or a component of only dots/spaces must
   never produce a path outside the unpack target; the entry is skipped with a warning, the rest is extracted.
   (Task 1.1 tests every form.)
2. A crafted rapified config whose class body offset points at itself or an ancestor must fail with an error, not
   overflow the stack or run for minutes. (Task 1.2 `rapified_self_reference_is_an_error`.)
3. The cache must rebuild when `scripts.pbo` changes (size or mtime) and must not be served for the other branch:
   a stable index is never answered for `--experimental`. (Task 2.4 `cache_rebuilds_on_pbo_change`,
   `branches_have_separate_caches`.)
4. MCP stdout carries only JSON-RPC frames: no command may `println!` while the server runs; warnings go into the
   tool result. (Task 3.1 removes the prints; Task 3.2 `mcp_stdout_is_pure_jsonrpc`.)
5. Two concurrent `dayzmake api` runs building the same cache must not leave a truncated index: writes go to a temp
   file in the cache dir and are renamed into place. (Task 2.4 `cache_write_is_atomic`.)

---

## Task 1: Untrusted input

### Task 1.1: Safe PBO unpacking

**Files:**
- Modify: `src/pbo.rs` (`cmd_unpack`, new `safe_relative_path`)
- Modify: `src/run.rs` (usage line + `--use-prefix` flag for unpack)
- Test: `tests/pbo.rs`

**Interfaces:**
- Produces: `pub fn safe_relative_path(name: &str) -> Option<PathBuf>` in `src/pbo.rs` — splits on `\` and `/`,
  drops empty components, returns `None` if any component is `.`/`..`, consists only of dots and spaces, contains
  `:`, a NUL or any ASCII control char, or if no component remains. Used again by Task 2 (cache/`--cat` never needs
  it, but Task 3's `pbo_unpack` goes through `cmd_unpack`).
- Produces: `pub fn cmd_unpack<I: Read>(input: &mut I, output: &Path, force: bool, use_prefix: bool) -> Result<(), Error>`.
  With `use_prefix` the files land in `<output>/<prefix as path>/<entry>`; the prefix goes through
  `safe_relative_path` too (invalid prefix → error, nothing written).

- [ ] **Step 1: Write the failing tests** in `tests/pbo.rs`. Helper `fn crafted_pbo(entries: &[(&str, &[u8])], prefix: Option<&str>) -> Vec<u8>`:
  build with `PBO::from_directory` on an empty temp dir, then clear `files`, insert the entries and set
  `header_extensions["prefix"]`, `to_cursor()`. Tests (all run the CLI via the existing `run_dayzmake` helper and
  check nothing exists outside `target`, using a sibling dir `outside/` next to `target/`):
  - `unpack_rejects_parent_dir_entry`: entry `..\\outside\\evil.txt` → `outside/evil.txt` absent, stderr contains
    `unsafe-path`, exit 0, a second normal entry `ok.txt` is extracted.
  - `unpack_rejects_absolute_entry`: entries `\\abs.txt` is extracted as `target/abs.txt` (leading separator
    dropped), `/tmp/x` likewise `target/tmp/x`; `C:\\x.txt` skipped.
  - `unpack_rejects_dot_space_components`: entries `a\\...\\b`, `a\\. \\b`, `a\\.\\b` skipped.
  - `unpack_use_prefix_rejects_dotdot_prefix`: prefix `..\\..\\outside` with `--use-prefix` → non-zero exit, error
    mentions the prefix, nothing written anywhere.
  - `unpack_use_prefix_extracts_below_prefix`: prefix `my\\mod`, entry `a\\b.c` → `target/my/mod/a/b.c`.
  - `unpack_does_not_follow_symlinks` (`#[cfg(unix)]`): pre-create `target/link -> ../outside`; entry
    `link\\evil.txt` with `--force` → `outside/evil.txt` absent, warning printed.
  - `unpack_prefix_file_strips_control_chars`: header extension value `"x\nprefix=..\\evil"` → written
    `$PBOPREFIX$` has exactly one line per extension and no line `prefix=..\evil` (newlines/CR replaced by a space,
    warning `unsafe-header`).
  - `unpack_duplicate_target_warns`: entries `A.txt` and `a.txt` on a case-insensitive check: second one whose
    lowercased path was already written in this run is skipped with warning `duplicate-entry` (without `--force` it
    must not abort the whole unpack).
  - unit tests in `src/pbo.rs` `#[cfg(test)]` for `safe_relative_path` covering each rule above.
- [ ] **Step 2: Run** `cargo test --test pbo unpack_ && cargo test --lib safe_relative_path` → FAIL.
- [ ] **Step 3: Implement.** `cmd_unpack`: map each entry through `safe_relative_path` (skip + warning name
  `unsafe-path`); before writing, walk every component from `output` downwards with `symlink_metadata` and refuse
  (warning `unsafe-path`) if any existing component is a symlink; never open through an existing symlink even with
  `--force`; track written lowercase relative paths for `duplicate-entry`. `$PBOPREFIX$`: replace `\r`/`\n`/other
  control chars in keys and values with a space (warning `unsafe-header`). Add `[--use-prefix]` to the `unpack`
  usage line and `flag_use_prefix: bool` to `Args`; usage text: `--use-prefix   unpack: extract into <targetfolder>/<prefix>/.`
- [ ] **Step 4: Run** the tests → PASS; `make fmt lint test` → PASS.
- [ ] **Step 5: Commit** `pbo: never unpack outside the target folder`.

### Task 1.2: Bounded readers (no panic, no unbounded allocation/recursion)

**Files:**
- Modify: `src/pbo.rs` (`lzss_decompress`, `matches_glob`)
- Modify: `src/config.rs` (`ConfigClass::read_rapified`, `ConfigArray::read_rapified`)
- Modify: `src/paa.rs` (`Paa::read`)
- Test: `tests/config.rs`, `tests/paa.rs`, `tests/pbo.rs`, unit tests in the modules

**Interfaces:**
- Produces in `src/config.rs`: constants `MAX_RAPIFIED_DEPTH: u32 = 128`, `MAX_RAPIFIED_ENTRIES: u64 = 4_000_000`
  (total entries + array elements per file). `read_rapified` keeps its public signature; internally passes a
  `&mut ReadBudget { depth, remaining, open_bodies: Vec<u32> }`.
- Produces in `src/paa.rs`: `MAX_PAA_DIMENSION: usize = 16384`, `MAX_TAGG_SIZE: usize = 1 << 20`.

- [ ] **Step 1: Write the failing tests:**
  - `tests/config.rs::rapified_self_reference_is_an_error`: hand-build bytes `\0raP` + 4 zero bytes + `u32 8` +
    `u32 offset_of_end` at 0..16 (copy the header layout from `Config::write_rapified`), root body: parent `""`,
    1 entry, type 0, name `"a"`, classbody_fp = offset of the root body itself → `Config::read_rapified` returns
    `Err` (message contains `nesting` or `cycle`) within the test's normal runtime.
  - `rapified_deep_array_nesting_is_an_error`: an array with 10 000 nested type-3 elements → `Err`.
  - `rapified_exponential_fanout_is_an_error`: 30 class levels, each body with 2 class entries pointing at the same
    next body → `Err` mentioning `too many entries` (2^30 would otherwise be read).
  - `tests/pbo.rs::compressed_entry_size_bomb_is_an_error`: a PBO whose header says packing `0x43707273`,
    `original_size = 0xFFFF_FFF0`, `data_size = 16` → `dayzmake cat` exits non-zero with an error, no OOM.
    (Build by writing a normal 1-entry PBO and patching the header u32s.)
  - unit `matches_glob` tests: `("", "*")`, `("ab", "abc*")`, `("é", "*x")`, `("abc", "a*c")`, `("abc", "*")` —
    no panic; results `true,false,false,true,true`. `*` matches any run including empty.
  - `tests/paa.rs::paa_huge_tagg_is_an_error` (TAGG size `0xFFFF_FFFF`) and `paa_huge_dimensions_is_an_error`
    (first mipmap `w = 0x7FFF`, `h = 0xFFFF`, LZO flag set) → `Paa::read` returns `Err`.
- [ ] **Step 2: Run** those tests → FAIL (some may abort the test binary; that counts as failing).
- [ ] **Step 3: Implement.** Config: error if depth > `MAX_RAPIFIED_DEPTH`, if a class body offset is already in
  `open_bodies` (`"Rapified class body cycle at offset N."`), or when `remaining` hits 0
  (`"Rapified config has too many entries."`). LZSS: reject `original_size > data.len().saturating_mul(9)` before
  allocating (max expansion of the format is < 9x); cap `with_capacity` at `data.len() * 9`. `matches_glob`:
  rewrite iteratively over `char`s (classic `*`-only wildcard with backtracking), no slicing. PAA: read TAGG data via
  `take(...).read_to_end` after rejecting sizes > `MAX_TAGG_SIZE`; reject mipmap width/height > `MAX_PAA_DIMENSION`
  and LZO `decompressed_size > mip_data.len().saturating_mul(256)`.
- [ ] **Step 4: Run** the tests → PASS; `make fmt lint test` → PASS.
- [ ] **Step 5: Commit** `bound allocation and recursion when reading untrusted files`.

### Task 1.3: Security audit of all untrusted-input code + mutation test

**Files:**
- Create: `tests/untrusted.rs`
- Create: `docs/security-review-2026-11.md`
- Modify: whatever the audit finds (expected candidates: `src/p3d.rs`, `src/sign.rs`, `src/io.rs`, `src/paa.rs`,
  `src/binarize.rs` path handling, `src/preprocess.rs` include resolution)

- [ ] **Step 1: Write `tests/untrusted.rs`:** a deterministic xorshift64 PRNG (seed constant `0x9E37_79B9_7F4A_7C15`),
  for each seed file — every file in `tests/fixtures/bi/**` with extension `pbo|bin|paa|p3d|bikey|bisign|biprivatekey`
  plus `tests/fixtures/mlod/small.p3d` — 300 mutants (1–8 random byte flips, plus truncation at a random length in
  1/4 of mutants). Feed each to the matching reader in-process (`PBO::read` + `file_data` of every entry,
  `Config::read_rapified`, `Paa::read`, `p3d::P3D::read` / `p3d::model_magic`, `BIPublicKey::read`,
  `BIPrivateKey::read`, `BISign::read`). Test passes if no panic; each `Result` is just dropped.
  Test `mutants_do_not_panic`. Add `#[ignore]`d `mutants_do_not_panic_long` with 20 000 mutants per file.
- [ ] **Step 2: Run** `cargo test --release --test untrusted` and the ignored one once
  (`cargo test --release --test untrusted -- --ignored`). Fix every panic/abort found (each fix gets a regression
  case in the same file using the exact failing bytes).
- [ ] **Step 3: Audit by reading.** Go through every reader of untrusted data in `src/` (pbo, config rapified
  reader, paa, p3d, sign, io, binarize output handling, preprocess includes) and check: unchecked arithmetic on
  sizes/offsets from the file, allocations sized by the file, recursion, seeks to file-supplied offsets, slicing,
  paths built from file content, files written outside an intended dir, temp files created predictably, external
  processes started with file-supplied arguments. Write findings to `docs/security-review-2026-11.md`: one line per
  finding: `file:line — issue — fixed in <commit/this task> | accepted because <reason>`. Fix every issue that is not
  explicitly accepted, each with a test.
- [ ] **Step 4: Run** `make check` → PASS.
- [ ] **Step 5: Commit** `security review of untrusted-input readers`.

---

## Task 2: Script API index (`dayzmake api`)

Index format: dayzmake's own (Task 2.2 model, cached as JSON). No compatibility with the dayz-dev plugin's `.txt.xz` indexes or their text format (user ruling 2026-11-14).

### Task 2.1: Steam/DayZ install discovery

**Files:**
- Create: `src/api/mod.rs` (module docs + `pub mod` lines; `pub mod api;` in `src/lib.rs`)
- Create: `src/api/steam.rs`
- Test: unit tests in `src/api/steam.rs` with fake Steam roots in temp dirs

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Branch { Stable, Experimental }` with
    `fn client_app(self) -> u32` (221100 / 1024020), `fn server_app(self) -> u32` (223350 / 1042420),
    `fn name(self) -> &'static str` (`"stable"` / `"experimental"`).
  - `pub fn steam_roots() -> Vec<PathBuf>` — `$STEAM_ROOT` first, then the roots listed in `find-dayzserver.sh`
    (built from `$HOME`), deduplicated by canonical path, only existing ones with a `steamapps` dir.
  - `pub fn libraries(roots: &[PathBuf]) -> Vec<PathBuf>` — each root plus every `"path"` value in
    `steamapps/libraryfolders.vdf`, deduplicated.
  - `pub struct AppInstall { pub dir: PathBuf, pub buildid: Option<String> }`
  - `pub fn find_app(libraries: &[PathBuf], appid: u32) -> Option<AppInstall>` — first library with
    `steamapps/appmanifest_<appid>.acf` whose `installdir` dir exists under `steamapps/common`.
  - `pub fn find_game(branch: Branch, libraries: &[PathBuf]) -> Option<AppInstall>` — client app first, then server
    app, first one that has `dta/scripts.pbo`.
  - `pub fn workshop_dir(branch: Branch, libraries: &[PathBuf]) -> Option<PathBuf>` —
    `steamapps/workshop/content/<client_app>`.
  - `fn vdf_values<'a>(text: &'a str, key: &str) -> Vec<&'a str>` — the `"key" "value"` lines, like `vdf_value` in
    the shell script; unescape `\\` to `\`.

- [ ] **Step 1: Tests:** `finds_stable_client`, `finds_experimental_client_not_stable` (both manifests present,
  each branch picks its own app), `falls_back_to_server`, `reads_library_folders` (game in a second library listed in
  `libraryfolders.vdf`), `buildid_is_read`, `missing_install_is_none`, `vdf_ignores_malformed_lines` (unterminated
  quotes, binary garbage → no panic, no value).
- [ ] **Step 2: Run** `cargo test --lib api::steam` → FAIL.
- [ ] **Step 3: Implement** per the Interfaces block.
- [ ] **Step 4: Run** → PASS; `make fmt lint test`.
- [ ] **Step 5: Commit** `api: find the DayZ install and workshop dir per branch`.

### Task 2.2: Index model and Enforce Script declaration parser

User ruling (2026-11-14): no compatibility with the Python index files or their text format is needed; design the
index for Rust and dayzmake's own readers. The Python scripts remain a reference for *what* is extracted (classes,
bases, modded parts, members with modifiers, `//!` docs, enums, typedefs, top-level functions/vars, defines), not for
how it is stored or printed.

**Files:**
- Create: `src/api/model.rs` (data model), `src/api/parse.rs` (scanner + declaration parser)
- Modify: `src/api/mod.rs` (`pub mod model; pub mod parse;`), `Cargo.toml` (`serde_json = "1"`)
- Fixtures: `tests/fixtures/api/vanilla/scripts/**` and `tests/fixtures/api/mod/**` (synthetic `.c` files already in
  the working tree, untracked, from an aborted attempt — review, extend, commit them; no game data)
- Test: `tests/api.rs`

**Interfaces:**
- Produces in `src/api/model.rs` (all `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`):
  - `pub struct ApiIndex { pub format: u32, pub name: String /* "vanilla" or mod name */, pub title: String, pub build: String, pub inputs: Vec<InputStamp>, pub modules: Vec<Module>, pub classes: BTreeMap<String, Class>, pub enums: BTreeMap<String, Enum>, pub typedefs: Vec<Member>, pub functions: Vec<Member>, pub globals: Vec<Member>, pub defines: BTreeMap<String, Vec<String>> }`
    `pub const INDEX_FORMAT: u32 = 1;` (bump on any model change; the cache treats other values as stale).
  - `pub struct InputStamp { pub path: PathBuf, pub size: u64, pub mtime_ns: u128 }` (filled by Task 2.4).
  - `pub struct Module { pub prefix: String /* PBO prefix, `\`-separated */, pub pbo: String /* "dta/scripts.pbo" or "addons/x.pbo" */ }`
  - `pub struct FileRef { pub module: usize /* index into modules */, pub path: String /* entry name inside the PBO, `\`-separated */, pub line: u32 /* 1-based line of the declaration */ }`
  - `pub struct Class { pub name: String, pub type_params: Vec<String>, pub base: Option<String>, pub alt_bases: Vec<String>, pub modded_only: bool, pub doc: Option<String>, pub decls: Vec<FileRef>, pub members: Vec<Member> }`
    (`decls[0]` = main declaration; modded parts merged in; `modded_only` = every declaration is `modded class`.)
  - `pub enum MemberKind { Method, Field }`
  - `pub struct Member { pub kind: MemberKind, pub name: String, pub signature: String, pub modifiers: Vec<String>, pub doc: Option<String>, pub obsolete: bool, pub decl: FileRef }`
    — `signature` is the normalized declaration without body (whitespace collapsed, `ref/autoptr/owned/notnull/reference`
    dropped, `proto` shown as `native`, keyword order as in the Python `member()`); `modifiers` holds the kept
    keywords (`static override private protected native const sealed abstract final event? ...` — same keep/drop sets
    as `MODS`/`DROP` in `build-api-index.py`). `private` members are dropped, as in the reference.
  - `pub struct Enum { pub name: String, pub base: Option<String>, pub decls: Vec<FileRef>, pub items: Vec<EnumItem> }`,
    `pub struct EnumItem { pub name: String, pub value: Option<i64>, pub doc: Option<String> }` (implicit values
    computed like the reference; non-numeric expression → `None`).
- Produces in `src/api/parse.rs`:
  - `pub struct ScriptFile<'a> { pub module: usize, pub path: &'a str /* entry name in the PBO */, pub text: &'a str }`
  - `pub fn active_defines(branch: Branch) -> BTreeSet<String>` — the reference's `ACTIVE` set; for both branches.
  - `pub fn index_scripts(files: &[ScriptFile<'_>], defines: &BTreeSet<String>, into: &mut ApiIndex)` — parses each
    file and merges classes/enums/top-level decls into `into` (callers set name/title/build/modules).
  - `pub fn collect_defines(config_text: &str, scripts: &[ScriptFile<'_>]) -> Vec<String>` — `defines[]` arrays of a
    derapified/plain `config.cpp` plus `#define NAME` lines, deduplicated in first-seen order.
- Parser rules (decide behavior, not code): a scanner that understands `//`, `/* */`, `//!`, `/*! */` and `/** */`
  docs (first sentence, copyright headers dropped as in the reference's `scan`), string literals with escapes, and
  preprocessor lines. `#ifdef/#ifndef/#else/#endif` are evaluated against `defines` plus `#define`s seen earlier in
  the same file; inactive text is skipped, but a `class X extends Y` header in an inactive branch adds `Y` to
  `alt_bases` of `X`. Statements split at brace depth 0 as in the reference's `split` (initializer lists like
  `int a[] = {1, 2};` are fields). Layer of a file = its `1_Core`..`5_Mission` path component (not stored separately;
  it is in `FileRef.path`). Files are processed in a deterministic order: sorted by (layer, module, path).
  Never panic and never recurse unboundedly on hostile input (unterminated comments/strings, unbalanced braces,
  10 000 nested braces): such input yields a best-effort partial result.

- [ ] **Step 1: Tests** in `tests/api.rs` that parse the fixture tree with `index_scripts` (helper reads the fixture
  files into `ScriptFile`s, module 0, `path` = path below `scripts/` with `\`): assert on the model, e.g.
  `parses_class_with_base_and_members` (exact `signature`, `modifiers`, `doc`, `decl.line`),
  `modded_parts_are_merged` (`decls.len() == 2`, members from both, `modded_only == false`),
  `modded_only_class`, `ifdef_variant_picks_active_base_and_records_alt`, `private_members_dropped`,
  `override_and_proto_native_modifiers`, `obsolete_attribute`, `template_params`, `enum_values_implicit_explicit_hex`,
  `enum_item_docs`, `typedefs_functions_globals`, `initializer_list_is_a_field`, `strings_with_braces_and_comments`,
  `copyright_header_is_not_a_doc`, `crlf_line_numbers`, `deterministic_order`,
  `hostile_input_does_not_panic` (unterminated `/*`, unterminated string, 10 000 `{`, lone `#endif`, NUL bytes),
  `collect_defines_from_config_and_scripts`, `index_roundtrips_through_json` (`serde_json` to string and back equals).
- [ ] **Step 2: Run** `cargo test --test api` → FAIL.
- [ ] **Step 3: Implement.** Plain `&str`/byte scanning; `regex` only where it clearly simplifies (static regexes in
  `LazyLock` like `src/pbo.rs`).
- [ ] **Step 4: Run** → PASS; `make fmt lint test`. Also time `index_scripts` on the real vanilla scripts once
  (manual, read `dta/scripts.pbo` of the stable install with `PBO::read`): report count of classes and the duration
  in the task report; target < 2 s in release.
- [ ] **Step 5: Commit** `api: index model and Enforce Script declaration parser`.

### Task 2.3: Query engine

**Files:**
- Create: `src/api/query.rs`
- Modify: `src/api/mod.rs`, `tests/api.rs`

**Interfaces:**
- Consumes: `ApiIndex` from Task 2.2.
- Produces:
  - `pub enum Query { Class { name: String, inherited: bool, mods_only: bool }, Find { regex: String, class: Option<String> }, Sub { name: String, all: bool }, Enum { name: String }, Chain { name: String }, Src { name: String }, Defines { regex: Option<String> } }`
  - `pub struct SrcFile { pub index: String /* ApiIndex.name */, pub pbo: String, pub prefix: String, pub path: String, pub line: u32 }`
  - `#[derive(Debug, Serialize)] pub enum QueryResult { Classes(Vec<ClassView>), Matches(Vec<Match>), Names(Vec<String>), Chain(Vec<String>), Enums(Vec<EnumView>), Sources(Vec<SrcFile>), Defines(Vec<DefineSet>) }`
    — owned data carrying the index name next to the model data: `ClassView { index: String, class: Class }`,
    `Match { index: String, scope: String /* class name, "fn", "var", "typedef" or enum name */, member: Option<Member>, item: Option<EnumItem>, class_name_match: bool }`,
    `EnumView { index: String, en: Enum }`, `DefineSet { index: String, prefix: String, names: Vec<String> }`.
  - `pub fn run_query(indexes: &[ApiIndex], query: &Query) -> Result<QueryResult, Error>` — `indexes[0]` is vanilla.
  - `pub fn render_text(result: &QueryResult, multi: bool, max: usize) -> Vec<String>` — the text format below,
    capped at `max` lines with a final `... N more lines (raise --max or narrow the query)`.
  - `pub fn render_json(result: &QueryResult, max: usize) -> String` — `{"result": <QueryResult>, "truncated": N}`
    via `serde_json`; items beyond `max` are dropped and counted in `truncated` (0 when complete).
  - `pub fn src_files(indexes: &[ApiIndex], name: &str) -> Vec<SrcFile>` (class or enum; every declaration).
- Output format (decided here; compact, one declaration per line, readable by humans and LLMs):
  - class header: `class Name<T> : Base  [vanilla 3_Game]` (tag = index name, layers of its declarations;
    `[modded]` appended when `modded_only`; tag shown only when more than one index is loaded); then
    `  // <doc>` if any, `  // base is X on a stable server, Y when its #ifdef define is off` if `alt_bases`;
    members `  <signature>  // <doc>` (fields first, then methods, each sorted case-insensitively by name);
    overriding methods (`override` modifier) collapse into one line `  override: A B C`.
  - `find`: `Class: <signature>` per matching member (regex case-insensitive, over signature and doc), plus class
    names and enum items that match, plus top-level decls as `fn: ...`, `var: ...`, `typedef: ...`.
  - `sub`: class names separated by spaces (direct, or transitive with `all`); `chain`: `A > B > C`.
  - `enum`: header `enum Name : Base` then `  NAME = value  // doc`.
  - `src`: `[<index>] <pbo> <prefix>\<path>:<line>` per declaration.
  - `defines`: `[<index>] <prefix>: NAME NAME ...` filtered by regex.
  - Not found → `Err` with a hint: `no class "X"; try: dayzmake api find X` plus, when only vanilla is loaded,
    `  (searched vanilla only; add -m <mod> or -m all for mods)`. Invalid regex → `Err`, never a panic.
- [ ] **Step 1: Tests** in `tests/api.rs` on indexes built from the fixtures (vanilla + the fixture mod as a second
  index): one test per query kind and option (`inherited`, `mods_only`, `class` filter, `all`, `max` cap line),
  asserting exact `render_text` lines; `json_output_parses_and_has_members`, `json_truncation_count` for
  `render_json`; `query_not_found_hint`, `find_invalid_regex_is_an_error`,
  `chain_stops_on_cycle` (classes A : B, B : A → terminates).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS; `make fmt lint test`.
- [ ] **Step 5: Commit** `api: query the script API index`.

### Task 2.4: Sources from PBOs, cache, CLI

**Files:**
- Create: `src/api/source.rs`, `src/api/cache.rs`
- Modify: `src/api/mod.rs` (high-level entry points), `src/run.rs` (usage + dispatch), `README.md` (section
  "Script API index"), `tests/api.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: Tasks 2.1–2.3, `PBO::read` / `PBO::file_data`, `Config::read_rapified` + `Config::write` (derapify to
  text), `pbo::safe_relative_path`.
- Produces in `src/api/source.rs`:
  - `pub enum ModSpec { Alias(&'static str, u64), Workshop(u64), Dir(PathBuf) }`, `pub fn parse_mods(spec: &str) -> Result<Vec<ModSpec>, Error>`
    — comma list; aliases `cf`→1559212036, `cot`→1564026768, `dabs`→2545327648, `expansion`→2572331007; `all` = all
    aliases that are installed; digits = workshop id; anything else = directory.
  - `pub struct ModSource { pub name: String /* alias, or dir name sanitized to [A-Za-z0-9_.-] */, pub dir: PathBuf, pub pbos: Vec<PathBuf> /* dir/addons/*.pbo, case-insensitive "addons", sorted */ }`
  - `pub fn resolve_mod(spec: &ModSpec, branch: Branch, libraries: &[PathBuf]) -> Result<ModSource, Error>`
  - `pub fn build_vanilla(scripts_pbo: &Path, branch: Branch, build: &str) -> Result<ApiIndex, Error>` — reads the
    PBO once, indexes its `.c` entries in memory (module 0 = prefix `scripts`, pbo `dta/scripts.pbo`).
  - `pub fn build_mod(m: &ModSource, branch: Branch) -> Result<ApiIndex, Error>` — one module per PBO (prefix from the
    PBO header; a prefix or entry name failing `safe_relative_path` → that PBO/entry skipped with a warning; missing
    prefix → file stem); `.c` entries plus `config.bin` (derapified) / `config.cpp` for `defines`, keyed by prefix.
    Title: `name = "..."` from `<dir>/meta.cpp` if present, else the dir name, plus ` (workshop <id>)`.
  - Parse PBOs in parallel with `std::thread::scope` (one thread per PBO, bounded by
    `std::thread::available_parallelism`) only if Task 2.2's timing shows vanilla > 1 s; otherwise sequential.
- Produces in `src/api/cache.rs`:
  - `pub fn cache_dir() -> Result<PathBuf, Error>` — `$DAYZMAKE_CACHE`, else `$XDG_CACHE_HOME/dayzmake/api`, else
    `$HOME/.cache/dayzmake/api` (Windows: `%LOCALAPPDATA%\dayzmake\api`); created with mode 0700 on unix.
  - `pub fn stamps(inputs: &[PathBuf]) -> Result<Vec<InputStamp>, Error>`
  - `pub fn cached(key: &str, inputs: &[PathBuf], refresh: bool, build: impl FnOnce() -> Result<ApiIndex, Error>) -> Result<ApiIndex, Error>`
    — loads `<dir>/<key>.json` if it parses, `format == INDEX_FORMAT`, and its `inputs` equal `stamps(inputs)`;
    else builds, sets `inputs`, writes JSON to a temp file in the same dir and `rename`s it into place. A corrupt or
    foreign cache file is rebuilt, never an error. `key` must match `[A-Za-z0-9_.-]+`.
  - keys: vanilla `vanilla-<branch>`; mod `mod-<name>-<branch>-<first 8 hex of SHA-1 of the canonical mod dir>`.
- Produces in `src/api/mod.rs`:
  - `pub struct ApiContext { pub branch: Branch, pub dayz: Option<PathBuf>, pub mods: String, pub refresh: bool }`
  - `pub fn load_indexes(ctx: &ApiContext) -> Result<Vec<ApiIndex>, Error>` — stale caches rebuilt automatically on
    every call; vanilla build string `"<branch> build <buildid>"` (or `"<branch> build unknown"`).
  - `pub fn cat_sources(ctx: &ApiContext, name: &str) -> Result<String, Error>` — for every `SrcFile`:
    `// ==== [<index>] <pbo> <prefix>\<path>:<line>\n` + the full file text read from that PBO, blank line between.
- CLI (add to `USAGE`; `--class <class>` instead of `-c` because `-c` is `--compress`):
  ```
  dayzmake api class [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--inherited] [--mods-only] <name>
  dayzmake api find [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--class <class>] <regex>
  dayzmake api sub [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--all] <name>
  dayzmake api enum [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] <name>
  dayzmake api chain [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] <name>
  dayzmake api src [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--cat] <name>
  dayzmake api defines [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [<regex>]
  dayzmake api update [--experimental] [--dayz <dir>] [-m <mods>]
  ```
  `--max` default 120. `--dayz <dir>`: a DayZ client/server install dir (uses `<dir>/dta/scripts.pbo`), skips Steam
  discovery for vanilla. `api update` forces a rebuild and prints `<key>: <classes> classes, <build>` per index.
  Every query command also takes `[--json]` (add it to each `api` usage line except `update`): prints
  `render_json` instead of `render_text`; errors stay on stderr as text. Query errors exit 1 with the message on
  stderr. `--verbose` prints `# <index>: <build>` lines first (text mode only).

- [ ] **Step 1: Tests** in `tests/api.rs` (fixture PBOs built at test time with `PBO::from_directory` from the
  fixture trees; a fake Steam root with `appmanifest_221100.acf` (`buildid 111`) and `appmanifest_1024020.acf`
  (`buildid 222`) whose `dta/scripts.pbo` differ in one class; `DAYZMAKE_CACHE` and `STEAM_ROOT` set on the spawned
  CLI process only): `api_class_from_pbo_stable_default`, `api_experimental_uses_exp_install`,
  `branches_have_separate_caches`, `cache_reused_when_unchanged` (cache file mtime unchanged on the 2nd run),
  `cache_rebuilds_on_pbo_change`, `cache_write_is_atomic` (no temp files left; a corrupt `.json` is replaced),
  `cache_format_bump_rebuilds`, `api_src_cat_prints_full_file` (contains a method body line from the fixture),
  `api_src_shows_line_numbers`, `api_class_json` (stdout parses as JSON, members present), `api_mod_by_dir`, `api_mod_unsafe_prefix_skipped` (prefix `..\\evil` → warning, other
  PBO indexed, nothing written outside the cache dir), `api_missing_install_message` (exit 1, stderr names the branch
  and `--dayz`); `tests/cli.rs::cli_usage_lists_api`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement**; printing only in `run.rs`.
- [ ] **Step 4: Run** → PASS; `make check`. Manual smoke on the real install:
  `cargo run --release -q -- api class PlayerBase --max 5`, same with `--experimental`, `api src AIAgent --cat | head`,
  `api find GetGroup -m cf`; report first-run (cache build) and second-run times in the task report.
- [ ] **Step 5: Commit** `api: cached script API index from the installed PBOs`.

## Task 3: MCP server (`dayzmake mcp`)

### Task 3.1: Commands write to a caller-supplied writer; warnings can be captured

**Files:**
- Modify: `src/pbo.rs` (`cmd_inspect`, `cmd_cat`), `src/p3d.rs` (`cmd_inspect`), `src/error.rs`, `src/run.rs`
- Test: existing CLI tests must still pass unchanged; new unit tests in `src/error.rs`

**Interfaces:**
- Produces:
  - `pub fn pbo::cmd_inspect<I: Read, O: Write>(input: &mut I, output: &mut O) -> Result<(), Error>`
  - `pub fn p3d::cmd_inspect<I: Read + Seek, O: Write>(input: &mut I, output: &mut O) -> Result<(), Error>`
  - `pbo::cmd_cat`: entry not found → `Err("File \"<name>\" not found in PBO.")` (CLI exits 1 instead of printing
    `not found` and exiting 0; update any test that relied on the old behavior).
  - `pub fn error::capture_warnings<T>(f: impl FnOnce() -> T) -> (T, Vec<String>)` — while `f` runs, warnings are
    collected (formatted like the printed line, without colors) instead of printed to stderr; per-name quotas are
    reset before `f`.
  - No `println!`/`print!` left in `src/` outside `src/run.rs` (grep check in Step 4).

- [ ] **Step 1: Tests:** `capture_warnings_collects_and_does_not_print`, `capture_warnings_resets_quota`;
  `tests/pbo.rs::cat_missing_entry_fails`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.** In `run.rs` pass `&mut stdout()`.
- [ ] **Step 4: Run** `make check` → PASS and `grep -n 'println!\|print!(' src/*.rs src/api/*.rs | grep -v '^src/run.rs'`
  → no output.
- [ ] **Step 5: Commit** `commands write to a caller-supplied writer; capturable warnings`.

### Task 3.2: stdio MCP server

**Files:**
- Create: `src/mcp.rs` (`pub mod mcp;` in `src/lib.rs`)
- Modify: `Cargo.toml` (`serde_json = "1"`), `src/run.rs` (`dayzmake mcp [-v]` usage line, Commands text
  `mcp         Run an MCP server on stdin/stdout (tools for the API index and the file commands).`),
  `README.md` (section "MCP server" with a `claude mcp add dayzmake -- dayzmake mcp` example)
- Test: `tests/mcp.rs`

**Interfaces:**
- Consumes: everything above. Tools never call `print_error`, never exit; each call runs inside
  `capture_warnings` and ends with `binarize::cleanup()`.
- Produces: `pub fn mcp::serve<R: BufRead, W: Write>(input: R, output: W) -> Result<(), Error>` — newline-delimited
  JSON-RPC 2.0. Methods: `initialize` (reply `protocolVersion`: echo the client's if it is `2025-06-18`,
  `2025-03-26` or `2024-11-05`, else `2025-06-18`; `capabilities: {"tools": {}}`; `serverInfo: {name: "dayzmake",
  version}`), `notifications/initialized` (ignored), `ping` (`{}`), `tools/list`, `tools/call`; unknown method →
  error `-32601`; malformed JSON → error `-32700` with `id: null`; notifications (no `id`) never get a reply.
  Tool results: `{"content":[{"type":"text","text": <output>}], "isError": bool}`; output = command text, then
  `\n\nwarnings:\n` + captured warnings if any; text over 200 000 bytes is cut with a final line
  `[truncated: N bytes total]`. Tool failures are results with `isError: true`, not JSON-RPC errors.
- Tools (name — arguments; `required` in the JSON schema marked *):
  - `api_class` — *name, inherited, mods_only, mods, experimental, max
  - `api_find` — *regex, class, mods, experimental, max
  - `api_subclasses` — *name, all, mods, experimental, max
  - `api_enum` — *name, mods, experimental
  - `api_chain` — *name, mods, experimental
  - `api_source` — *name, cat (bool: return the full files), mods, experimental
  - `api_defines` — regex, mods (default `all`), experimental
  - `api_update` — mods, experimental
  - `pbo_inspect` — *path (PBO or P3D; same detection as the CLI `inspect`)
  - `pbo_cat` — *pbo, *file, target (write there instead of returning; required if the data is not UTF-8), force
  - `pbo_unpack` — *pbo, *target, force, use_prefix
  - `pbo_pack` / `pbo_build` — *source, *target, exclude (array), headerext (array), include (array, build only),
    key (private key: sign after packing), force, proton_binarize (build only)
  - `config_rapify` — *source, *target, include (array), force
  - `config_derapify` — *source, target (absent → text returned)
  - `config_preprocess` — *source, target, include (array)
  - `paa_to_image` — *source, *target, force; `image_to_paa` — *source, *target, type (default `dxt5`), compress, force
  - `key_generate` — *name (path without extension), force
  - `pbo_sign` — *private_key, *pbo, signature, v2, force; `pbo_verify` — *public_key, *pbo, signature
  - `binarize` — *source, *target, force (sets proton mode on non-Windows, like `--proton-binarize`)
  All paths are taken as given (relative to the server's cwd); no tool overwrites an existing file unless `force`.

- [ ] **Step 1: Tests** in `tests/mcp.rs` (spawn `CARGO_BIN_EXE_dayzmake mcp`, write requests, read lines):
  `mcp_initialize_and_list_tools` (all 22 tool names above present, each with `inputSchema.type == "object"`),
  `mcp_stdout_is_pure_jsonrpc` (call `pbo_inspect` on a fixture PBO and `pbo_unpack` that triggers a warning;
  every stdout line parses as JSON with `"jsonrpc":"2.0"`; the warning text is inside the result),
  `mcp_unknown_method_is_32601`, `mcp_parse_error_is_32700`, `mcp_notification_gets_no_reply` (send
  `notifications/initialized` then `ping` id 7 → the first reply line has id 7), `mcp_tool_error_is_result`
  (`pbo_inspect` on a missing file → `isError: true`, process still alive for the next request),
  `mcp_api_class_with_fake_steam` (reuse the Task 2.4 fixture builder; set `STEAM_ROOT`/`DAYZMAKE_CACHE` on the
  child), `mcp_pbo_cat_binary_requires_target`, `mcp_unpack_dotdot_entry_stays_inside` (Task 1.1 crafted PBO).
- [ ] **Step 2: Run** `cargo test --test mcp` → FAIL.
- [ ] **Step 3: Implement** `src/mcp.rs`: one `fn tools() -> serde_json::Value` (the static list with schemas),
  one `fn call_tool(name: &str, args: &serde_json::Map<String, Value>) -> Result<String, Error>` dispatching to the
  library functions with an in-memory `Vec<u8>` as output; argument helpers `str_arg`, `bool_arg`, `strs_arg`
  returning `Error` for missing/wrongly typed required arguments.
- [ ] **Step 4: Run** → PASS; `make check`; `cargo deny check` accepts serde_json.
- [ ] **Step 5: Commit** `mcp: stdio MCP server for the API index and the file commands`.

---

## Task 4: Query all installed mods (user request 2026-11-14, runs after Task 3)

### Task 4.1: Discover every installed mod and query them together

**Files:**
- Modify: `src/api/source.rs`, `src/api/mod.rs`, `src/api/cache.rs` (if needed), `src/run.rs`, `src/mcp.rs`
  (`mods` argument docs + new `api_mods` tool), `README.md`
- Test: `tests/api.rs`, `tests/mcp.rs`

**Interfaces:**
- Consumes: `ModSpec`/`parse_mods`/`resolve_mod`/`build_mod`/`cached`/`load_indexes` (Task 2.4), `workshop_dir`,
  `find_game` (Task 2.1), MCP tool table (Task 3.2).
- Produces in `src/api/source.rs`:
  - `pub struct InstalledMod { pub id: Option<u64>, pub name: String /* meta.cpp name, else dir name */, pub dir: PathBuf, pub has_scripts: bool }`
  - `pub fn installed_mods(branch: Branch, libraries: &[PathBuf], game: Option<&Path>) -> Vec<InstalledMod>` — every
    dir in the branch's workshop content dir plus every `@*` dir in the game install dir (DayZ launcher links), deduplicated by
    canonical path (symlinks resolved), sorted by name; `has_scripts` = any `addons/*.pbo` (case-insensitive) whose
    header lists a `.c` entry (read headers only via `PBO::read`; unreadable PBO → warning, not an error).
  - `parse_mods` changes: `all` = every installed mod with `has_scripts` (not just the four aliases); new
    `aliases` = the four aliases (old `all` behavior); a token that is neither alias, digits, nor an existing path
    is matched case-insensitively against `InstalledMod.name` and the `@dir` name (unique match required, else an
    error listing up to 10 candidates).
- Building many mods: indexes not in the cache are built in parallel with `std::thread::scope`, at most
  `available_parallelism()` threads; a mod that fails to build is skipped with a warning (the query still runs).
  Freshness checks stay per mod (stat only). Index names must be unique: on a name clash append `-<workshop id>`.
- CLI: `dayzmake api mods [--experimental] [--dayz <dir>] [--json]` lists installed mods: `<id or ->  <name>  <dir>  [scripts]`.
  The `-m all` help text says it may take a while the first time (every mod is indexed once, then cached).
- MCP: new tool `api_mods` (experimental) and the `mods` argument description of every api tool mentions `all`,
  `aliases`, ids, names and dirs.

- [ ] **Step 1: Tests:** fake Steam root with a workshop dir holding 3 mods (2 with script PBOs, 1 textures-only) and
  a game dir with one `@link` symlink to one of them (`#[cfg(unix)]`): `installed_mods_dedups_links`,
  `installed_mods_flags_scripts`, `mods_all_queries_every_scripted_mod` (a class from each mod found by `api find`),
  `mods_by_name_case_insensitive`, `mods_ambiguous_name_is_an_error`, `mods_aliases_keeps_old_all`,
  `broken_mod_is_skipped_with_warning` (garbage `.pbo`), `index_name_clash_gets_id_suffix`, `cli_api_mods_lists`,
  `cli_api_mods_json`, `mcp_api_mods_tool`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS; `make check`. Manual smoke on the
  real install: `time dayzmake api mods | wc -l`, `time dayzmake api find '^OnRPC' -m all --max 20` twice (first
  run builds caches, second must be fast); report both times and the mod count in the task report.
- [ ] **Step 5: Commit** `api: query all installed mods`.

## Final: whole-branch review (orchestrator, not a subagent task)

- Review `git diff main...feature/untrusted-api-mcp` against this plan and the Review Focus list, fix findings,
  run `make check`, manual smoke of `dayzmake api` and `dayzmake mcp` against the real install.
- Report to the user: anything in the dayz-dev plugin that should switch to `dayzmake api` / `unpack --use-prefix`
  (`extract-dayz-sources.sh` builds `$out/$prefix/$rel` from untrusted `inspect` output — the same `..` issue lives
  there; it is a different repository, so it is only reported, not changed).
