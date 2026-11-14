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

Index format: byte-for-byte the format written by `build-api-index.py` (read its header lines), except header line 1,
which becomes `# DayZ script API index, <build>. Generated by dayzmake <version> from the vanilla scripts.` (mods:
`# Script API index of the mod <name>, <title>. Generated by dayzmake <version> from the mod PBOs.`), and the text
`dayz-api.py` in header/error lines becomes `dayzmake api`. Cached files are plain text (`.txt`, not xz).

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

### Task 2.2: Index builder (port of `build-api-index.py`)

**Files:**
- Create: `src/api/index.rs`
- Create: `tests/fixtures/api/vanilla/scripts/{1_Core,3_Game,4_World,5_Mission}/**/*.c` (synthetic, small)
- Create: `tests/fixtures/api/mod/<two module dirs with layer dirs>/**/*.c`, a `config.cpp` with `defines[]`, a
  `pbo-map.tsv`
- Create: `tests/fixtures/api/expected/{dayz-api.txt,mod-fixture.txt}` (generated, see Step 1)
- Create: `tests/api.rs`

**Interfaces:**
- Produces:
  - `pub struct SourceTree { pub files: BTreeMap<String, String> }` — key: `/`-separated path relative to the index
    root (vanilla: relative to the `scripts` prefix, e.g. `3_Game/AI/AIAgent.c`; mod: `<prefix>/<inner path>`,
    e.g. `JM/CF/Scripts/3_Game/x.c`, plus derapified `<prefix>/config.cpp`), value: file text (UTF-8 lossy,
    like Python's `errors='replace'`). Includes only `.c` files and `config.cpp`.
  - `pub fn SourceTree::from_dir(root: &Path) -> Result<SourceTree, Error>` (tests and `--dayz`-style dirs).
  - `pub struct IndexOptions { pub module_name: Option<String>, pub build: String, pub pbo_map: BTreeMap<String, String> }`
    (`module_name` = `None` for vanilla; `pbo_map`: prefix with `/` → pbo path relative to the mod dir).
  - `pub fn build_index(tree: &SourceTree, opts: &IndexOptions) -> String` — the whole index text, `\n` line ends,
    trailing newline.

- [ ] **Step 1: Make the golden files.** Write the fixtures so they exercise: `//!` and `/*! */` docs (incl. a
  copyright header to drop), `modded class`, `#ifdef FEATURE_NETWORK_RECONCILIATION class A extends B #else class A
  extends C #endif`, templates `class X<Class T>`, `proto native`, `private` members (dropped), `override` methods,
  `[Obsolete]` attribute, enums with explicit/implicit/hex values and per-item docs, typedefs, top-level functions
  and vars, initializer lists `int a[] = {1,2};`, strings containing `{`, `;` and `//`. Generate the expected files
  with the reference script (scratch dir under /tmp):
  ```bash
  S=/home/bzed/workspace/skills/dayz-dev-plugin/scripts; T=$(mktemp -d /tmp/apigold.XXXXXX)
  python3 $S/build-api-index.py tests/fixtures/api/vanilla/scripts $T --build "fixture build 1"
  python3 $S/build-api-index.py tests/fixtures/api/mod $T --mod fixture --build "Fixture Mod 1.0"
  xz -dc $T/dayz-api.txt.xz > tests/fixtures/api/expected/dayz-api.txt
  xz -dc $T/mod-fixture.txt.xz > tests/fixtures/api/expected/mod-fixture.txt
  ```
  Then apply the header-line-1 and `dayz-api.py`→`dayzmake api` substitutions described above by hand (document
  them in a comment in `tests/api.rs`), so the expected files are what dayzmake must emit.
- [ ] **Step 2: Tests** in `tests/api.rs`: `index_matches_reference_vanilla`, `index_matches_reference_mod`
  (`build_index(&SourceTree::from_dir(...)?, ...)` == expected file, compare line-by-line and print the first
  differing line on failure; the `<version>` in line 1 is `env!("CARGO_PKG_VERSION")`). Run → FAIL.
- [ ] **Step 3: Implement** `src/api/index.rs` as a faithful port: `scan`, `split`, `clean`, `short`, `member`,
  `class_variants`, `parse_file`, `module_ids`, `collect_defines`, output assembly. Notes: Rust `regex` has no
  look-behind — implement `re.split(r'(?<=[.!?])\s|...')` by hand; the `VARIANT` regex in the Python file is unused,
  skip it; keep Python's sort orders exactly (`sorted(classes)` is by code point; members by `(kind == 'm',
  name.lower())`, stable). `ACTIVE` defines set is the stable one from the script for both branches.
  Python's `os.walk` sorting: dirs and files sorted by name at each level, layer dirs iterated in `sorted(LAYERS)` —
  with a `BTreeMap` the order per layer must be reproduced by walking, not by sorting full paths (a file `a.c` sorts
  before dir `a/b.c` only if that is what `os.walk` does: files of a dir come before its subdirs). Defines in the
  mod: every `config.cpp` (`defines[] = {...}`) and `#define NAME` lines in `.c` files, grouped per prefix from
  `pbo_map` (longest prefix match), as in `collect_defines`.
- [ ] **Step 4: Run** → PASS; `make fmt lint test`.
- [ ] **Step 5: Commit** `api: build the script API index (port of build-api-index.py)`.

### Task 2.3: Query engine (port of `dayz-api.py`)

**Files:**
- Create: `src/api/query.rs`
- Create: `tests/fixtures/api/expected/queries/*.txt` (generated)
- Modify: `tests/api.rs`

**Interfaces:**
- Consumes: index text from Task 2.2.
- Produces:
  - `pub struct LoadedIndex { pub name: String /* "vanilla" or the mod name */, pub title: String, pub modules: BTreeMap<String, (String, String)> /* id → (prefix, pbo) */, blocks: ... }`
    with `pub fn LoadedIndex::parse(name: &str, text: &str) -> LoadedIndex` (port of `load` for one file).
  - `pub enum Query { Class { name: String, inherited: bool, mods_only: bool }, Find { regex: String, class: Option<String> }, Sub { name: String, all: bool }, Enum { name: String }, Chain { name: String }, Src { name: String }, Defines { regex: Option<String> } }`
  - `pub struct SrcFile { pub index: String, pub pbo: String /* "dta/scripts.pbo" for vanilla, relative to mod dir otherwise */, pub inner: String /* backslash path inside the pbo, without prefix */ }`
  - `pub fn run_query(indexes: &[LoadedIndex], query: &Query, max: usize) -> Result<Vec<String>, String>` — the
    printed lines of `dayz-api.py` (including the `... N more lines (raise --max or narrow the query)` cap line);
    `Err` carries the `sys.exit` message text. `indexes[0]` is vanilla.
  - `pub fn src_files(indexes: &[LoadedIndex], name: &str) -> Vec<SrcFile>` — used by `api src --cat` (Task 2.4)
    and MCP.
  - `hint` text (vanilla only) becomes `  (searched vanilla only; add -m <mod> or -m all for mods)`.

- [ ] **Step 1: Golden outputs.** Put the two Task 2.2 expected index files (xz-compressed) into
  `$T/api/dayz-api.txt.xz` and `$T/api/mod-fixture.txt.xz`, copy `dayz-api.py` to `$T/scripts/`, and record for
  each case `python3 $T/scripts/dayz-api.py <args> > tests/fixtures/api/expected/queries/<case>.txt 2>&1`; cases:
  `class <C>`, `class <C> --inherited`, `class <C> -m fixture`, `class <C> -m fixture --mods-only`, `find <rx>`,
  `find <rx> -c <C>`, `sub <C>`, `sub <C> --all`, `enum <E>`, `chain <C>`, `src <C> -m fixture`, `defines -m fixture`,
  `class Missing`, `find . --max 3`. Apply the `dayz-api.py`→`dayzmake api` / hint substitution by hand.
- [ ] **Step 2: Tests** `query_<case>` in `tests/api.rs`, one per golden file, comparing `run_query` output (or the
  `Err` text) joined with `\n`. Plus `find_invalid_regex_is_an_error` (`find "("` → `Err`, no panic). Run → FAIL.
- [ ] **Step 3: Implement** the port. Regex: Rust `regex` with `(?i)` prefix for `find`/`defines`.
- [ ] **Step 4: Run** → PASS; `make fmt lint test`.
- [ ] **Step 5: Commit** `api: query the script API index (port of dayz-api.py)`.

### Task 2.4: Sources from PBOs, cache, CLI

**Files:**
- Create: `src/api/source.rs`, `src/api/cache.rs`
- Modify: `src/api/mod.rs` (high-level entry points), `src/run.rs` (usage + dispatch), `README.md` (new section
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
  - `pub fn vanilla_tree(scripts_pbo: &Path) -> Result<SourceTree, Error>` — only `.c` entries of the PBO whose
    name passes `safe_relative_path`, keyed `/`-separated.
  - `pub fn mod_tree(m: &ModSource) -> Result<(SourceTree, BTreeMap<String, String>), Error>` — per PBO: prefix
    (normalized to `/`, must pass `safe_relative_path`, else the PBO is skipped with a warning; missing → pbo file
    stem); `.c` entries and `config.bin` (derapified) / `config.cpp`; returns the tree and the pbo map
    (prefix → `addons/<file>.pbo`). Prefix and module id strings containing whitespace are skipped with a warning
    (they would break the index line format).
  - title for a mod: `name = "..."` from `<dir>/meta.cpp` if present, else the dir name, plus
    ` (workshop <id>)` for workshop mods.
- Produces in `src/api/cache.rs`:
  - `pub fn cache_dir() -> Result<PathBuf, Error>` — `$DAYZMAKE_CACHE`, else `$XDG_CACHE_HOME/dayzmake/api`, else
    `$HOME/.cache/dayzmake/api` (Windows: `%LOCALAPPDATA%\dayzmake\api`); created with mode 0700 on unix.
  - `pub fn stamp(inputs: &[PathBuf], extra: &str) -> Result<String, Error>` — lines
    `format=1`, `dayzmake=<version>`, `extra=<extra>`, then `<size>\t<mtime ns>\t<path>` per input.
  - `pub fn cached(key: &str, stamp: &str, build: impl FnOnce() -> Result<String, Error>) -> Result<String, Error>`
    — reads `<dir>/<key>.txt` if `<dir>/<key>.stamp` equals `stamp`; else builds, writes both via temp file in the
    same dir + `rename` (index first, stamp last). `key` must match `[A-Za-z0-9_.-]+`.
  - keys: vanilla `vanilla-<branch>`; mod `mod-<name>-<branch>-<first 8 hex of SHA-1 of the canonical mod dir>`.
- Produces in `src/api/mod.rs`:
  - `pub struct ApiContext { pub branch: Branch, pub dayz: Option<PathBuf>, pub mods: String, pub refresh: bool }`
  - `pub fn load_indexes(ctx: &ApiContext) -> Result<(Vec<LoadedIndex>, Vec<PathBuf> /* pbo per index: vanilla scripts.pbo, mod dirs */), Error>`
    — stale caches rebuilt automatically on every call; `refresh` forces a rebuild. Vanilla build string:
    `"<branch> build <buildid>"` (or `"<branch> build unknown"`).
  - `pub fn cat_sources(ctx: &ApiContext, name: &str) -> Result<String, Error>` — for every `SrcFile` of `name`,
    `// ==== [<index>] <pbo> <inner>\n` followed by the full file text from the PBO (vanilla: `scripts.pbo` entry
    `<inner>`; mods: `<mod dir>/<pbo>`), files separated by a blank line.
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
  `--max` default 120. `--dayz <dir>`: a DayZ client/server install dir (uses `<dir>/dta/scripts.pbo`) — skips
  Steam discovery for vanilla. `api update` forces a rebuild and prints one line per index:
  `<key>: <classes> classes, <path>`. Query errors exit 1 with the message on stderr. Each query prints a first
  line `# <branch> <build>` only with `--verbose`.

- [ ] **Step 1: Tests:**
  - `tests/api.rs` (build fixture PBOs at test time with `PBO::from_directory` from `tests/fixtures/api/vanilla/scripts`
    with `$PBOPREFIX$` `scripts`, and a fake Steam root with `appmanifest_221100.acf` (`buildid 111`) and
    `appmanifest_1024020.acf` (`buildid 222`) whose `dta/scripts.pbo` differ in one class; set
    `DAYZMAKE_CACHE` and `STEAM_ROOT` on the spawned CLI process, never on the test process):
    `api_class_from_pbo_stable_default`, `api_experimental_uses_exp_install`, `branches_have_separate_caches`,
    `cache_reused_when_unchanged` (second run does not rewrite `.stamp`: compare mtime),
    `cache_rebuilds_on_pbo_change` (rewrite scripts.pbo with an extra class, sizes differ → new class visible),
    `cache_write_is_atomic` (no `*.tmp*` left in the cache dir; a pre-existing truncated `.txt` with a wrong stamp
    is replaced), `api_src_cat_prints_full_file` (output contains a method body line from the fixture),
    `api_mod_by_dir` (`-m <fixture mod dir>` built into a temp `addons/` with two PBOs),
    `api_mod_unsafe_prefix_skipped` (a mod PBO with prefix `..\\evil` → warning, other PBO indexed, nothing written
    outside the cache dir), `api_missing_install_message` (empty Steam root → exit 1, stderr names the branch and
    `--dayz`).
  - `tests/cli.rs::cli_usage_lists_api`.
- [ ] **Step 2: Run** `cargo test --test api --test cli` → FAIL.
- [ ] **Step 3: Implement** per Interfaces; dispatch in `run.rs` calls `api::load_indexes` + `run_query` and prints
  lines (one `#[expect(clippy::print_stdout, reason = "CLI command output")]`).
- [ ] **Step 4: Run** → PASS; `make check`. Manual smoke (not a test, real install present on this machine):
  `cargo run -q -- api class PlayerBase --max 5`, `cargo run -q -- api class PlayerBase --experimental --max 5`,
  `cargo run -q -- api src AIAgent --cat | head`, `cargo run -q -- api find GetGroup -m cf`. Compare
  `cargo run -q -- api class PlayerBase --max 10000` against
  `python3 ~/workspace/skills/dayz-dev-plugin/scripts/dayz-api.py class PlayerBase --max 10000` (same build) — only
  the substituted text may differ. Report the diff in the task summary.
- [ ] **Step 5: Commit** `api: cached script API index from the installed PBOs`.

---

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

## Final: whole-branch review (orchestrator, not a subagent task)

- Review `git diff main...feature/untrusted-api-mcp` against this plan and the Review Focus list, fix findings,
  run `make check`, manual smoke of `dayzmake api` and `dayzmake mcp` against the real install.
- Report to the user: anything in the dayz-dev plugin that should switch to `dayzmake api` / `unpack --use-prefix`
  (`extract-dayz-sources.sh` builds `$out/$prefix/$rel` from untrusted `inspect` output — the same `..` issue lives
  there; it is a different repository, so it is only reported, not changed).
