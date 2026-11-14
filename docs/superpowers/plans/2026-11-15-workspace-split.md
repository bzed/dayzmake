# dayzmake Workspace Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split dayzmake into a reusable library crate `dayzmake-core` (all file formats, signing, binarize bridge)
and the `dayzmake` CLI/MCP crate on top of it, with byte-identical CLI and MCP behaviour, proven by a
characterization suite recorded before the refactor.

**Architecture:** The repo root stays the `dayzmake` package and becomes the workspace root; the library moves to
`crates/dayzmake-core`. The library loses all printing and global state: warnings go to a caller-supplied
`WarningSink`, binarization goes through a `Binarizer` value, CLI helpers (`cmd_*`, stdin/stdout, `--force`
checks) move to the CLI crate. Public types get accessors and options structs instead of public maps and
CLI-shaped arguments.

**Tech Stack:** Rust 1.99, edition 2024, Cargo workspace, `insta` (dev, snapshots), `cargo llvm-cov`,
`cargo deny`, `cargo fuzz`.

**Spec:** `docs/superpowers/specs/2026-11-15-workspace-split-design.md`

## Global Constraints

- All rules in `AGENTS.md` apply (lints, no panics on untrusted input, checked arithmetic, `#[expect]` with reason,
  `error!`/`prepend_error`, tests under `tempfile::TempDir`, `HOME`/`XDG_*` set to temp dirs in CLI tests).
- `make check` passes after every task. Run it with `CARGO_TARGET_DIR` under `/var/tmp` (the Makefile default).
- **No-breakage rule:** from Task 3 on, existing tests and snapshots may change only imports, paths and call
  signatures, never expected values or snapshot contents. Exceptions are listed per task (Task 3, Task 5) and the
  commit message names every snapshot it changes. `cargo insta` must never be run with `--accept` on unrelated
  snapshots; review each `.snap.new` by hand.
- Coverage (`make coverage-check`, workspace-wide) stays at or above 70 % in every task.
- `dayzmake-core`: version `0.1.0`, licence `GPL-2.0-or-later`, no dependency on `docopt`, `colored`, `serde`,
  `serde_json`; no `print!`/`eprint!` anywhere in it (no `#[expect(clippy::print_*)]` either).
- `dayzmake` (root): version stays `2.0.0` (no release in this plan); depends on
  `dayzmake-core = { path = "crates/dayzmake-core", version = "0.1.0" }`.
- Workspace-internal path dependencies (with a `version`) are the only allowed path dependencies; AGENTS.md says so
  after Task 4.
- Every new dependency is reviewed per AGENTS.md *Dependencies*; the review goes into the commit message.
- No game data in the repo. The differential run (Task 13) reads the user's DayZ install at run time and writes only
  under `/var/tmp`.
- Every crate linked into `dayzmake-core` must be licensed compatibly with both GPL-2.0-or-later and AGPL-3.0
  (no GPL-2.0-only code): consumers such as Náves are AGPL-3.0.
- Commit prefixes: `test:`, `fix:`, `build:`, `refactor:`, `feat:`, `docs:`, `ci:`. Commits end with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Work on a branch (`feature/workspace-split`), not `main`.

## Review Focus

1. **Parallel use from one process** (Náves converts tiles on many threads): warnings of one call must never show up
   in another call's sink, and nothing may serialise on a global lock. Pinned in Task 5
   (`warnings_stay_with_their_own_sink_across_threads`) and Task 12 (`naves_workflow_in_parallel`).
2. **Proton sandbox cleanup on the error path:** the CLI exits through `process::exit` in `print_error`, which skips
   `Drop`. The `Binarizer` must be dropped before that. Pinned in Task 6
   (`build_failure_with_proton_removes_the_sandbox`).
3. **stdin/stdout commands** (`rapify`, `derapify`, `preprocess`, `inspect`, `cat` without file arguments) move with
   `Input`/`Output` to the CLI crate; easy to break silently. Pinned in Task 1 (`*_stdin_*` snapshots).
4. **Signing a PBO read from disk must keep using the stored checksum**, not a recomputed one, or signatures of
   PBOs with foreign trailers change. Pinned in Task 8 (`signing_a_read_pbo_uses_its_stored_checksum`).
5. **Hostile PBOs with duplicate header-extension keys** after the switch to an ordered list: must read without
   panic, keep one value per key and write deterministically. Pinned in Task 3
   (`duplicate_header_extension_keys_read_and_write_deterministically`).

---

## File Structure (end state)

```
Cargo.toml                       # [workspace] + package dayzmake (CLI, MCP, api)
src/
  main.rs  lib.rs  run.rs  mcp.rs  api/…        # unchanged roles
  cli_io.rs                      # Input, Output, create_target (from core io.rs)
  cli_warnings.rs                # PrintWarnings sink, summary printing, PrintError trait (print_error)
  commands/mod.rs                # pub mod config, pbo, p3d, paa, sign, binarize
  commands/{config,pbo,p3d,paa,sign,binarize}.rs   # former cmd_* functions
tests/                           # CLI/MCP/api tests, fixtures (shared), golden_cli.rs, golden_mcp.rs
crates/dayzmake-core/
  Cargo.toml  README.md  LICENSE
  src/lib.rs                     # crate docs with the consumer example (doctest)
  src/{pbo,config,preprocess,eval,io,paa,p3d,sign,binarize,error,warning}.rs
  tests/{config,preprocess,bi_golden,fuzz_seeds,warnings,pbo_api,paa_api,config_api,consumer}.rs
  benches/{preprocess,config}.rs
fuzz/                            # depends on both crates
testharness/compare-release.sh   # Task 13
```

---

### Task 1: Characterization snapshots of the CLI

Records how every CLI command behaves today. No product code changes.

**Files:**
- Create: `tests/golden_cli.rs`, `tests/snapshots/` (insta output), `tests/fixtures/golden/mod/` (inputs below),
  `tests/fixtures/golden/README.md`
- Modify: `Cargo.toml` (`[dev-dependencies]`), `tests/fixtures/bi/README.md` not touched

**Interfaces:**
- Produces: `tests/fixtures/golden/mod/` (reused by Tasks 2, 12, 13); helper
  `fn dayzmake(args: &[&str], cwd: &Path, stdin: Option<&[u8]>) -> Run` with
  `struct Run { code: i32, stdout: Vec<u8>, stderr: String }` (copy it into `golden_mcp.rs` if needed, no shared
  module required).

- [ ] **Step 1: Review and add `insta`**

`cargo add --dev insta --features json`. Review per AGENTS.md (licence Apache-2.0, maintenance, no build script
surprises) and put the review into the commit message. If it fails the review, write a ~30-line helper
`tests/common/golden.rs` instead (compare with a file under `tests/snapshots/`, rewrite it when `DAYZMAKE_BLESS=1`)
and say so in the commit. Binary outputs use `insta::assert_binary_snapshot!` with the file extension
(`".pbo"`, `".bin"`, `".paa"`, `".bisign"`).

- [ ] **Step 2: Create the inputs in `tests/fixtures/golden/mod/`** (all synthetic, written by hand)

- `$PBOPREFIX$`: one line, `golden\addon` (only the prefix, so the output does not depend on the header-extension
  order bug fixed in Task 3)
- `config.cpp`: `#include "inc\macros.hpp"`, a `CfgPatches` class with `units[]`/`requiredAddons[]` arrays, a
  function-like macro with arguments, `__EVAL(1+2)`, a float, a negative int, a nested class with inheritance, an
  external class declaration, a `delete` entry, an `#ifdef`/`#else` block, an unquoted string that raises the
  `unquoted-string` warning
- `inc/macros.hpp`: the macros used above
- `data/mat.rvmat`: a small material config
- `data/readme.txt`: plain text
- `data/sub/deep.txt`: plain text in a nested folder
- `README.md` in `tests/fixtures/golden/`: what the files are and that they are synthetic

The test copies the `mod/` tree into a temp dir per test and adds `tests/fixtures/mlod/small.p3d` as
`data/model.p3d`. It generates PNGs in the test: a 64×64 RGBA gradient (`r = x*4, g = y*4, b = 128, a = 255 - x*2`)
and a 64×64 opaque version (`a = 255`).

- [ ] **Step 3: Write the tests**

The helper sets `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME` to temp dirs, `NO_COLOR=1`, removes
`STEAM_ROOT` and `DAYZMAKE_*` from the environment, and runs with relative paths in a temp cwd. Text snapshots replace
the temp dir path with `[TMP]` and `\r\n` with `\n`. The file starts with `#![cfg(unix)]`: Linux records the
snapshots; Windows keeps its existing tests.

One test per line; every test asserts the exit code, and snapshots stdout and stderr (text) or the output file
(binary):

| Test | Command (cwd = temp copy of the mod) | Pins |
|---|---|---|
| `help_text` | `--help` | stdout snapshot (exact USAGE) |
| `version_text` | `--version` | `stdout == format!("v{}\n", env!("CARGO_PKG_VERSION"))` |
| `preprocess_file` | `preprocess config.cpp out.cpp` | `out.cpp` text, stderr |
| `preprocess_stdin_stdout` | `preprocess` with config bytes on stdin, `-i .` | stdout, stderr |
| `rapify_file` | `rapify config.cpp config.bin` | `config.bin` bytes, stderr (warnings) |
| `rapify_stdin_stdout` | `rapify -i .` with stdin | stdout bytes |
| `derapify_default` | `derapify config.bin out.cpp` (bin from `rapify_file`) | text |
| `derapify_indent` | `derapify -d "\t" config.bin out.cpp` | text |
| `derapify_stdin_stdout` | `derapify` with stdin | stdout |
| `pack_bytes` | `pack . ../out.pbo` | PBO bytes |
| `pack_headerext` | `pack -e product=golden . ../out.pbo` | PBO bytes |
| `pack_exclude` | `pack -x "*.txt" . ../out.pbo` | PBO bytes |
| `build_bytes_and_warnings` | `build . ../out.pbo` | PBO bytes, stderr (missing-texture and `non-windows-binarization` warnings) |
| `build_signed` | `build -k <fixture armk.biprivatekey> . ../out.pbo` | PBO bytes, `out.pbo.armk.bisign` bytes |
| `build_signed_custom_signature_path` | `build -k … -s ../sig.bisign . ../out.pbo` | `sig.bisign` exists and equals `build_signed`'s bytes |
| `inspect_pbo_file` / `inspect_pbo_stdin` | `inspect ../out.pbo` / stdin | stdout |
| `inspect_p3d` | `inspect data/model.p3d` | stdout |
| `unpack_tree` | `unpack ../out.pbo ../x` | snapshot of a sorted listing `path<TAB>len<TAB>first 16 bytes hex` of every file under `x/` |
| `unpack_use_prefix` | `unpack --use-prefix ../out.pbo ../x` | same listing |
| `unpack_existing_target` | `unpack` twice, then with `-f` | 2nd: exit 1 + stderr; 3rd: exit 0 |
| `cat_stdout` / `cat_to_file` / `cat_missing_entry` | `cat ../out.pbo data\readme.txt [t]`, unknown name | stdout / file / exit 1 + stderr |
| `keygen_sign_verify` | `keygen k`, `sign k.biprivatekey ../out.pbo`, `verify k.bikey ../out.pbo` | both key files exist; `.bikey` length snapshot; verify exit 0, empty stderr |
| `sign_v3_bytes` / `sign_v2_bytes` | `sign [--v2] <fixture key> ../out.pbo` | `.bisign` bytes |
| `verify_ok` | `verify <fixture armk.bikey> ../out.pbo` after `sign_v3` | exit 0 |
| `verify_tampered` | flip one data byte of the PBO, then verify | exit 1, stderr snapshot |
| `img2paa_dxt1` / `img2paa_dxt5` / `img2paa_rgba` | `img2paa -t <t> grad.png out.paa` | PAA bytes |
| `img2paa_default_type` | `img2paa grad.png out.paa` | equals `img2paa_dxt5` bytes |
| `img2paa_bad_type` | `img2paa -t bogus …` | exit 1, stderr |
| `paa2img_pixels` | `paa2img` of the `rgba` PAA to `back.png` | decoded RGBA8 pixels (binary snapshot `.rgba`), width/height |
| `binarize_without_proton` | `binarize data/model.p3d out.p3d` with empty `PATH` override for Steam (`STEAM_ROOT` set to an empty temp dir) | exit 1, stderr |
| `existing_targets_need_force` | for each of `rapify`, `derapify`, `preprocess`, `build`, `pack`, `keygen`, `sign`, `img2paa`, `paa2img`: run twice | 2nd run exit 1, stderr snapshot per command (`insta::assert_snapshot!(format!("exists_{cmd}"), …)`) |
| `missing_inputs` | `rapify nope.cpp x`, `inspect nope.pbo`, `unpack nope.pbo d` | exit 1, stderr per command |
| `warning_quota` | `build` of a dir with 12 files `c00.cpp`…`c11.cpp` that start with `\0raP` | stderr snapshot (today: 10 `prerapified-file` lines and **no** summary line; Task 5 changes this snapshot on purpose) |
| `warning_mute` | same with `-w prerapified-file` | stderr empty of that warning |
| `warning_verbose` | same with `-v` | 12 lines |
| `parse_error_output` | `rapify` of `class A { x = ; };` | exit 1, stderr snapshot (caret line) |

- [ ] **Step 4: Record and review**

Run: `INSTA_UPDATE=always cargo test --test golden_cli --locked` and then
`cargo test --test golden_cli --locked` (with `INSTA_UPDATE=no` set).
Expected: second run PASS. Read every `.snap` file: each must show plausible output, no absolute paths, no ANSI
escapes. Run `cargo test --test golden_cli` three times: PASS every time (determinism).

- [ ] **Step 5: `make check`, then commit**

```bash
git add Cargo.toml Cargo.lock tests/golden_cli.rs tests/snapshots tests/fixtures/golden
git commit -m "test: characterization snapshots for every CLI command"
```

---

### Task 2: Characterization snapshots of the MCP server

**Files:**
- Create: `tests/golden_mcp.rs` (+ snapshots)

**Interfaces:**
- Consumes: `tests/fixtures/golden/mod/` (Task 1). Talks to `dayzmake mcp` over stdin/stdout like `tests/mcp.rs`
  does (copy its session helper).

- [ ] **Step 1: Write the tests**

| Test | Request | Pins |
|---|---|---|
| `tools_list_schema` | `initialize`, `tools/list` | `insta::assert_json_snapshot!` of the full `tools` array |
| `file_tools_replies` | one `tools/call` each for `pbo_build`, `pbo_pack`, `pbo_inspect`, `pbo_cat`, `pbo_unpack`, `pbo_sign`, `pbo_verify`, `config_preprocess`, `config_rapify` (to a target file), `config_derapify`, `image_to_paa`, `paa_to_image`, `key_generate`, `binarize` (no Proton: error) on the golden mod | the `result` JSON per tool, temp paths replaced by `[TMP]`, a snapshot per tool |
| `warnings_reset_per_call` | `pbo_build` of the 12-prerapified-files dir twice | both replies have the same warning text (the quota restarts per call) |
| `error_reply_for_missing_file` | `pbo_inspect` of a missing path | the error reply JSON |

- [ ] **Step 2: Record, review, rerun** as in Task 1 Step 4. Expected: PASS, three runs identical.

- [ ] **Step 3: `make check`, then commit** — `test: characterization snapshots for the MCP server`

---

### Task 3: Fix — header extensions keep their file order

**Files:**
- Modify: `src/pbo.rs` (`PBO.header_extensions`, `HeaderBlock.extensions`, `PboSelection.header_extensions`,
  `parse_prefix_file`, `write`, `write_prefix_file`, `cmd_pack`, `cmd_build`), `src/api/mod.rs` and `src/mcp.rs`
  where they read the field
- Test: `tests/pbo.rs`, `tests/untrusted.rs`

**Interfaces:**
- Produces: `PBO.header_extensions: LinkedHashMap<String, String>` (and the same type in `PboSelection`); insertion
  order = order in the file / `$PBOPREFIX$`, then `-e` options. Writing: `prefix` first, then the rest in that order.
  A repeated key keeps its first position and its last value. Task 8 hides the field.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn header_extensions_keep_file_order_and_bytes_are_stable() {
    // $PBOPREFIX$: "prefix=t\nb=2\na=1\nc=3\nz=9\ny=8\n" + one data file
    // build 8 PBOs from the same folder in-process (PBO::from_directory + to_cursor)
    // assert: all 8 byte vectors are equal
    // assert: PBO::read(..).header_extensions keys == ["prefix", "b", "a", "c", "z", "y"]
}

#[test]
fn unpack_writes_prefix_file_in_original_order() {
    // pack the folder above, cmd_unpack it, read $PBOPREFIX$
    // assert_eq!(content, "prefix=t\nb=2\na=1\nc=3\nz=9\ny=8\n") — match the exact format
    // write_prefix_file produces today for a single prefix; check it first
}
```

In `tests/untrusted.rs`:

```rust
#[test]
fn duplicate_header_extension_keys_read_and_write_deterministically() {
    // hand-built PBO bytes: extensions "prefix","p","k","1","k","2" then one entry
    // assert: read succeeds, header_extensions == [("prefix","p"),("k","2")]
    // assert: writing it twice gives equal bytes
}
```

- [ ] **Step 2: Run:** `cargo test --test pbo --test untrusted --locked`. Expected: the first test fails (byte
  vectors differ or key order differs) and the duplicate-key test fails on the order assertion.

- [ ] **Step 3: Switch the three maps to `LinkedHashMap<String, String>`** (already a dependency). The duplicate-key
  rule above applies (`LinkedHashMap::insert` moves an existing key to the end, so check with `get_mut` first).

- [ ] **Step 4: Run:** `cargo test --workspace --locked`. Expected: PASS, and all Task 1/2 snapshots unchanged
  (they only use a prefix and at most one extra extension).

- [ ] **Step 4b: Sorted directory traversal.** `list_files` in `src/pbo.rs` returns `read_dir` order, so the
  order of build warnings (stderr, MCP replies) depends on the file system. Sort each directory's entries by name
  (byte order of the file name) before recursing. Test in `tests/cli.rs`:
  `build_warnings_come_in_sorted_file_order` (create `b.cpp`, `a.cpp`, `c.cpp` prerapified in that creation order;
  stderr lines name `a.cpp`, `b.cpp`, `c.cpp` in that order). Snapshot exception for this step only: the
  warning order in `build_bytes_and_warnings`, `warning_quota`, `warning_verbose` and `golden_mcp__pbo_build`
  may change to the sorted order; PBO bytes must not change (entries were already sorted on write).

- [ ] **Step 5: `make check`, commit** — `fix: write PBO header extensions in file order` (and a second commit
  `fix: traverse addon folders in sorted order` for Step 4b). The message names the
  rule (AGENTS.md *Same input, same output*) that was broken.

---

### Task 4: Workspace and the `dayzmake-core` crate (move only)

A mechanical move. Behaviour, public items and CLI stay the same.

**Files:**
- Create: `crates/dayzmake-core/Cargo.toml`, `crates/dayzmake-core/src/lib.rs`, `crates/dayzmake-core/LICENSE`
  (copy of `LICENSE`)
- Move with `git mv`: `src/{pbo,config,preprocess,eval,io,paa,p3d,sign,binarize,error}.rs` →
  `crates/dayzmake-core/src/`; `benches/*` → `crates/dayzmake-core/benches/`; `tests/{config,preprocess,bi_golden}.rs`
  → `crates/dayzmake-core/tests/`
- Split: `tests/fuzz_seeds.rs` → format seeds (`pbo_read`, `config_rapified`, `paa_read`, `p3d_read`, `sign_read`)
  to `crates/dayzmake-core/tests/fuzz_seeds.rs`; `api_parse`, `steam_vdf` stay in `tests/fuzz_seeds.rs`
- Modify: root `Cargo.toml`, `src/lib.rs`, `src/run.rs`, `src/mcp.rs`, `src/api/*.rs`, remaining `tests/*.rs`,
  `fuzz/Cargo.toml`, `fuzz/fuzz_targets/*.rs`, `fuzz/Cargo.lock`, `Makefile`, `.github/workflows/ci.yml`,
  `AGENTS.md` (layout table, toolchain, path dependency rule)

**Interfaces:**
- Produces: crate `dayzmake_core` with public modules `binarize, config, error, eval, io, p3d, paa, pbo,
  preprocess, sign` (same items as today) and the exported macro `dayzmake_core::error!`. Root crate `dayzmake`
  with `pub mod api, mcp, run`; it does **not** re-export core modules. Root code imports
  `dayzmake_core::{pbo, …}` and `use dayzmake_core::error;` (brings in module and macro).

- [ ] **Step 1: Workspace manifest**

Root `Cargo.toml`: add `[workspace] members = ["crates/dayzmake-core"]`, `resolver = "3"`,
`[workspace.package]` (`edition = "2024"`, `rust-version = "1.99"`, `license = "GPL-2.0-or-later"`, `homepage`,
`repository`, `authors`), move `[lints.*]` to `[workspace.lints.*]` and set `[lints] workspace = true` in both
crates. `[profile.release]` stays at the root. Core `Cargo.toml`: `name = "dayzmake-core"`, `version = "0.1.0"`,
`description = "Readers and writers for DayZ file formats: PBO, rapified config, P3D, PAA, BI keys and signatures"`,
the dependencies the moved modules use (`byteorder`, `openssl`, `linked-hash-map`, `regex`, `peg`, `image`,
`texpresso`, `lzo1x`, `encoding_rs`, `colored` (removed in Task 5), `winreg` for windows), dev-dependencies
`tempfile`, `criterion`, and the two `[[bench]]` entries. Root keeps `docopt`, `colored`, `serde`, `serde_json`,
`regex`, plus whatever the compiler asks for, and `cargo add --path crates/dayzmake-core` (then set
`version = "0.1.0"` next to the path).

- [ ] **Step 2: Fixture paths in moved tests**

Core tests find shared fixtures with
`Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")` and corpora with `"../../fuzz/corpus"`.
Doc examples in core change `dayzmake::` to `dayzmake_core::`; the `PBO::from_directory(PathBuf::from("src"), …)`
doctest still packs the crate's own `src`, so it keeps working.

- [ ] **Step 3: Makefile and CI**

`lint`: `cargo clippy --workspace --all-targets --locked -- -D warnings`; `doc`: `cargo doc --workspace --no-deps
--locked`; `test`: `cargo test --workspace --all-targets --locked` and `cargo test --workspace --doc --locked`;
`cover`/`coverage-check`: `cargo llvm-cov --workspace --all-features …`; `miri`: `--workspace`. Mirror the same
flags in `ci.yml` (lint job, linux and windows `cargo test --workspace --locked --no-fail-fast`). Release packaging
paths (`target/release/dayzmake`) stay valid. `fuzz/Cargo.toml` gets
`dayzmake-core = { path = "../crates/dayzmake-core" }`; format targets import `dayzmake_core`, `api_parse` and
`steam_vdf` keep `dayzmake`.

- [ ] **Step 4: AGENTS.md**

Layout table: add `crates/dayzmake-core/` (library), say which tests live where (binary-driven tests in `tests/`,
library tests in `crates/dayzmake-core/tests/`). *Dependencies*: "No git or path dependencies, except
workspace-internal crates (with a `version`)".

- [ ] **Step 5: Verify**

Run: `make check`. Expected: PASS; the number of tests run equals the count before the move
(record `cargo test --workspace -- --list 2>/dev/null | grep -c ': test$'` before and after: equal). All Task 1/2
snapshots unchanged. `make fuzz` builds every target.

- [ ] **Step 6: Commit** — `build: split into a workspace with the dayzmake-core library crate`

---

### Task 5: Per-call warnings instead of global warning state

**Files:**
- Create: `crates/dayzmake-core/src/warning.rs`, `crates/dayzmake-core/tests/warnings.rs`, `src/cli_warnings.rs`
- Modify: `crates/dayzmake-core/src/error.rs` (remove `WarningState`, `warning`, `warning_suppressed`,
  `capture_warnings`, `print_warning_summary`, `init_warnings`, `print_error`; drop `colored`: the parse-error caret
  becomes a plain `^`), every core function that raises warnings (`pbo`, `config`, `preprocess`, `p3d`,
  `binarize`), `src/api/{mod,cache,source}.rs`, `src/run.rs`, `src/mcp.rs`, core `Cargo.toml` (remove `colored`),
  `AGENTS.md` (*Concurrency*: remove the warning-counter exception; *Errors and output*: the library never prints)
- Test: `tests/golden_cli.rs` snapshot `warning_quota` changes on purpose

**Interfaces:**
- Produces (in `dayzmake_core::warning`):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Warning { pub name: Option<&'static str>, pub message: String, pub file: Option<String>, pub line: Option<u32> }
impl Warning {
    pub fn new(name: Option<&'static str>, message: impl Into<String>) -> Warning;
    pub fn in_file(self, file: impl Into<String>) -> Warning;
    pub fn at_line(self, line: u32) -> Warning;
    pub fn location(&self) -> String;   // "In file f:l: ", "In file f: ", "In line l: " or ""
}
impl std::fmt::Display for Warning;      // "{location}warning: {message}{ [name]}", exactly today's uncolored line

pub trait WarningSink {
    fn warn(&mut self, warning: Warning);
    fn is_suppressed(&self, name: Option<&str>) -> bool { false }
}

pub struct Warnings { /* muted: BTreeSet<String>, quota: u32, raised: BTreeMap<String, u32>, kept: Vec<Warning> */ }
impl Default for Warnings;               // quota 10 per name; unnamed warnings have no quota
impl Warnings {
    pub fn new() -> Warnings;
    pub fn mute(&mut self, name: impl Into<String>);
    pub fn unlimited(&mut self);
    pub fn record(&mut self, warning: Warning) -> Option<&Warning>;  // None if muted or over quota; counts every raised warning
    pub fn warnings(&self) -> &[Warning];
    pub fn into_warnings(self) -> Vec<Warning>;
    pub fn suppressed(&self) -> Vec<(String, u32)>;                  // (name, raised - quota) for names over quota, sorted by name, muted excluded
}
impl WarningSink for Warnings;           // warn = record; is_suppressed = muted or quota reached
```

- Every core function that warns gets a last parameter `warnings: &mut dyn WarningSink`:
  `preprocess::preprocess(input, origin, includefolders, warnings)`,
  `Config::read(input, path, includefolders, warnings)`, `Config::from_string(…, warnings)`,
  `PBO::from_directory(…, warnings)`, `pbo::cmd_unpack(…, warnings)` (until Task 7 moves it),
  `p3d::check_dependencies(…, warnings)`, `binarize::binarize(…, warnings)`, `binarize::cmd_binarize(…, warnings)`,
  and the `cmd_*` functions that call them. Root `api` functions that warn take it too.
- Root `src/cli_warnings.rs`: `pub struct PrintWarnings { inner: Warnings }` implementing `WarningSink` (records,
  then prints the kept warning immediately to stderr with a yellow bold `warning` label, same line layout);
  `impl PrintWarnings { pub fn new(muted: Vec<String>, verbose: bool) -> Self; pub fn print_summary(&self) }`; the
  summary text is today's text ("{n} warnings of type \"{name}\" were suppressed …" / singular form);
  `pub trait PrintError { fn print_error(self, exit: bool); }` for `Result<T, Error>` (prints `error: …` in red,
  then exits with 1 if `exit`; the caller prints the summary before).
- MCP: `run_tool` creates a `Warnings::new()` per call and formats kept warnings with `Display` for
  `with_warnings`.

- [ ] **Step 1: Write the failing tests** in `crates/dayzmake-core/tests/warnings.rs`

```rust
#[test] fn display_matches_the_cli_line() {
    assert_eq!(Warning::new(Some("x"), "m").in_file("f.cpp").at_line(3).to_string(), "In file f.cpp:3: warning: m [x]");
    assert_eq!(Warning::new(None, "m").to_string(), "warning: m");
    assert_eq!(Warning::new(None, "m").at_line(2).to_string(), "In line 2: warning: m");
}
#[test] fn quota_keeps_ten_and_counts_the_rest() {
    // 12 × record(Warning::new(Some("n"), "m")) → warnings().len() == 10, suppressed() == [("n".into(), 2)]
}
#[test] fn muted_names_are_dropped_and_not_summarised() { /* mute("n"); 12 records → 0 kept, suppressed() empty */ }
#[test] fn unlimited_keeps_everything() { /* unlimited(); 12 → 12 kept, suppressed() empty */ }
#[test] fn unnamed_warnings_have_no_quota() { /* 12 × name None → 12 kept */ }
#[test] fn preprocess_reports_into_the_given_sink() {
    // preprocess a string with a macro arity mismatch into `let mut w = Warnings::new()`
    // assert: one warning, name Some("macro-arity") (use today's name), line set
}
#[test] fn warnings_stay_with_their_own_sink_across_threads() {
    // 8 threads; thread i preprocesses a source with i+1 warning-raising lines into its own Warnings
    // (vary the warning count per thread; stay under the quota)
    // assert: thread i's sink holds exactly i+1 warnings, all with its own file name
}
```

In `tests/golden_cli.rs` nothing new: the `warning_quota` snapshot will change in Step 4.

Add to `tests/cli.rs`:

```rust
#[test] fn suppressed_warnings_are_summarised() {
    // build of 12 prerapified .cpp files: stderr contains exactly 10 "prerapified-file" lines and
    // "2 warnings of type \"prerapified-file\" were suppressed to prevent spam. Use \"-w prerapified-file\" to disable these warnings entirely."
}
```

- [ ] **Step 2: Run:** `cargo test -p dayzmake-core --test warnings --locked` → FAIL (module missing);
  `cargo test --test cli suppressed_warnings_are_summarised` → FAIL (no summary line today).

- [ ] **Step 3: Implement** the module, the parameter threading and the CLI/MCP sinks. `config.rs`'s
  `warning_suppressed` call becomes `warnings.is_suppressed(name)`. `run::args` builds one `PrintWarnings` per command,
  passes it down, prints the summary, then calls `print_error`.

- [ ] **Step 4: Verify**

Run: `cargo test --workspace --locked`. Expected: PASS except the `warning_quota` snapshot, whose new version
differs from the old one **only** by the added summary line. Accept exactly that snapshot (`cargo insta review`),
nothing else. The `parse_error_output` snapshot is unchanged (recorded with `NO_COLOR=1`). `tests/mcp.rs` and
`golden_mcp.rs`: unchanged.
Run: `grep -rn 'static\|LazyLock\|eprint\|print!' crates/dayzmake-core/src/error.rs crates/dayzmake-core/src/warning.rs`
→ no matches. `cargo tree -p dayzmake-core -e normal | grep colored` → no output.

- [ ] **Step 5: `make check`, commit** — `refactor: pass warnings per call instead of global state` (body: fixes the
  missing suppression summary; names the changed `warning_quota` snapshot; the parse-error caret is no longer red).

---

### Task 6: `Binarizer` instead of `set_proton` and the global sandbox

**Files:**
- Modify: `crates/dayzmake-core/src/binarize.rs`, `crates/dayzmake-core/src/pbo.rs` (`from_directory`),
  `src/run.rs`, `src/mcp.rs`
- Test: `crates/dayzmake-core/src/binarize.rs` (`mod tests`), `tests/cli.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BinarizeBackend { Native, Proton }   // Native: Windows registry lookup; Proton: unix only

pub struct Binarizer { /* backend, #[cfg(unix)] sandbox: Option<proton::Sandbox> */ }
impl Binarizer {
    pub fn new(backend: BinarizeBackend) -> Result<Binarizer, Error>;   // Native on non-Windows / Proton on Windows → Err naming the platform and `--proton-binarize`
    pub fn binarize(&mut self, input: &Path, addon_root: &Path, prefix: &str, warnings: &mut dyn WarningSink) -> Result<Vec<u8>, Error>;
}
impl Drop for Binarizer;                       // removes the Proton sandbox if one was created
```

  Removed: `set_proton`, `available`, `cleanup`, `PROTON_ENABLED`, `SANDBOX`. Kept: `is_binarized`,
  `binarized_description`. `PBO::from_directory(directory, binarize: bool, exclude, includefolders,
  binarizer: Option<&mut Binarizer>, warnings)`: with `binarize` and `None`, binarizable files are copied with the
  `non-windows-binarization` warning, exactly as today without `--proton-binarize`. (Task 7 folds `binarize` into
  `PackOptions`.)
- CLI: `run_command` creates `Some(Binarizer::new(Native))` on Windows, `Some(Binarizer::new(Proton))` with
  `--proton-binarize` on unix, else `None`; `binarize` command always uses Proton on unix (as today). The
  `Binarizer` lives inside `run_command`, so it is dropped before `print_error` exits. MCP: one `Binarizer` per
  `pbo_build`/`binarize` call.
- Windows `binarize`: replace the three `unwrap()`s on `input.parent()`/`file_name()`/`to_str()` with errors.

- [ ] **Step 1: Write the failing tests**

```rust
// binarize.rs mod tests (unix)
#[test] fn native_backend_is_refused_off_windows() {
    let err = Binarizer::new(BinarizeBackend::Native).err().unwrap();
    assert!(err.to_string().contains("--proton-binarize"));
}
#[test] fn dropping_a_binarizer_removes_its_sandbox() {
    // build a Binarizer with a Sandbox whose root is a temp dir child that exists; drop it; assert !root.exists()
}
```

```rust
// tests/cli.rs (unix)
#[test] fn build_failure_with_proton_removes_the_sandbox() {
    // fake Steam root (STEAM_ROOT) with a DayZ Tools dir and a `proton` script that exits 1;
    // TMPDIR set to a temp dir; `build --proton-binarize` of a folder with a .p3d
    // assert: exit code 1; no "dayzmake*" sandbox dir left under TMPDIR
}
```

  The sandbox location must follow `TMPDIR` for this test (check where `proton::Sandbox` creates its root; if it
  does not use `std::env::temp_dir()`, make it).

- [ ] **Step 2: Run** the three tests → FAIL (types missing / sandbox left over or global).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace --locked` → PASS, all snapshots unchanged.
  `grep -n 'static\|AtomicBool\|LazyLock' crates/dayzmake-core/src/binarize.rs` → no matches.
- [ ] **Step 5: `make check`, commit** — `refactor: explicit Binarizer instead of global Proton state`

---

### Task 7: CLI helpers move to the CLI crate; options structs; internals private

**Files:**
- Create: `src/commands/mod.rs`, `src/commands/{config,pbo,p3d,paa,sign,binarize}.rs`, `src/cli_io.rs`,
  `crates/dayzmake-core/tests/pbo_api.rs`
- Modify: all core modules, `src/run.rs`, `src/mcp.rs`, root tests that call `cmd_*` (`tests/pbo.rs` uses
  `cmd_sign`/`cmd_verify`: switch to `dayzmake::commands::sign::…`)

**Interfaces:**
- Moves unchanged in behaviour (signatures keep their parameters, plus `warnings` from Task 5):
  `config::{cmd_rapify, cmd_derapify}`, `preprocess::cmd_preprocess` → `commands::config`;
  `pbo::{cmd_inspect, cmd_cat, cmd_pack, cmd_build, cmd_unpack}` → `commands::pbo`;
  `p3d::cmd_inspect` → `commands::p3d`; `paa::{cmd_paa2img, cmd_img2paa}` → `commands::paa`;
  `sign::{cmd_keygen, cmd_sign, cmd_verify}` → `commands::sign`; `binarize::cmd_binarize` (and `cli_addon`) →
  `commands::binarize`; `io::{Input, Output, create_target}` → `src/cli_io.rs`.
- New in core:

```rust
// pbo.rs
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PackOptions { pub build: bool, pub exclude: Vec<String>, pub include_folders: Vec<PathBuf>, pub header_extensions: Vec<(String, String)> }
impl PackOptions { pub fn pack() -> PackOptions; pub fn build() -> PackOptions }
impl PBO {
    pub fn from_directory(directory: &Path, options: &PackOptions, binarizer: Option<&mut Binarizer>, warnings: &mut dyn WarningSink) -> Result<PBO, Error>;
}
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct UnpackOptions { pub force: bool, pub use_prefix: bool }
pub fn unpack<I: Read>(input: &mut I, target: &Path, options: &UnpackOptions, warnings: &mut dyn WarningSink) -> Result<(), Error>;
```

  `options.header_extensions` are applied after `$PBOPREFIX$` (same key: the option wins, as `-e` does today).
  The `key=value` parsing of `-e` and its error message stay in `commands::pbo`. `commands::pbo::cmd_unpack` is
  a thin wrapper over `pbo::unpack`.
- Become `pub(crate)` in core: module `eval`, module `io` (`ReadExt`, `WriteExt`, `compressed_int_len`),
  `preprocess::{preprocess_grammar, Definition, Directive, Macro, Token, Line, pathsep}`,
  `PreprocessInfo.line_origins` (the type stays public), `config::{EvalCtx, ConfigWarnings, config_grammar}`,
  `error::{PreprocessParseErrorExt, ConfigParseErrorExt}`. Stay public: `error!`, `ErrorExt::prepend_error`,
  `error::printable`, `pbo::safe_relative_path`, `p3d::{model_magic, lod_name, check_dependencies}`,
  `config::is_rapified`.

- [ ] **Step 1: Write the failing tests** in `crates/dayzmake-core/tests/pbo_api.rs`

```rust
#[test] fn pack_options_header_extensions_override_the_prefix_file() {
    // folder with $PBOPREFIX$ "prefix=a\nproduct=x\n"; options.header_extensions = [("product","y"),("version","1")]
    // assert read-back order/values: [("prefix","a"),("product","y"),("version","1")]
}
#[test] fn pack_options_exclude_skips_files() { /* exclude ["*.txt"] → no .txt entry */ }
#[test] fn build_option_rapifies_config_cpp() { /* PackOptions::build(), config.cpp → entry "config.bin" starting with b"\0raP" */ }
#[test] fn unpack_refuses_existing_files_without_force() { /* unpack twice: 2nd Err containing "already exists"; with force: Ok */ }
#[test] fn unpack_rejects_unsafe_entry_names() { /* hand-built PBO with entry "..\\evil.txt" → Err, nothing written outside target */ }
```

- [ ] **Step 2: Run** `cargo test -p dayzmake-core --test pbo_api --locked` → FAIL (types missing).
- [ ] **Step 3: Implement** the moves and the options API.
- [ ] **Step 4: Verify**

`cargo test --workspace --locked` → PASS, all snapshots unchanged.
`cargo tree -p dayzmake-core -e normal --prefix none | grep -E '^(docopt|colored|serde|serde_json) '` → no output.
`grep -rn 'fn cmd_\|Stdout\|stdin' crates/dayzmake-core/src` → no matches.
`make doc` → PASS (no broken intra-doc links after the moves).
- [ ] **Step 5: `make check`, commit** — `refactor: move CLI helpers out of dayzmake-core; options structs for pack and unpack`

---

### Task 8: PBO and signing API: accessors, sign without writing first

**Files:**
- Modify: `crates/dayzmake-core/src/pbo.rs`, `crates/dayzmake-core/src/sign.rs`, every user of the fields
  (`src/commands/*`, `src/api/*`, `src/mcp.rs`, tests, fuzz targets)
- Test: `crates/dayzmake-core/tests/pbo_api.rs`

**Interfaces:**
- Produces (fields of `PBO` and `PboSelection` become private):

```rust
impl PBO {
    pub fn new() -> PBO;                                                   // also Default
    pub fn entries(&self) -> impl Iterator<Item = (&str, &[u8])>;          // stored bytes, header order
    pub fn entry_names(&self) -> impl Iterator<Item = &str>;
    pub fn file_data(&self, name: &str) -> Result<Option<Vec<u8>>, Error>; // unchanged: decompressed
    pub fn insert_entry(&mut self, name: impl Into<String>, data: Vec<u8>) -> Result<(), Error>; // rejects names safe_relative_path rejects
    pub fn remove_entry(&mut self, name: &str) -> Option<Vec<u8>>;
    pub fn header_extensions(&self) -> impl Iterator<Item = (&str, &str)>;  // file order
    pub fn header_extension(&self, key: &str) -> Option<&str>;
    pub fn set_header_extension(&mut self, key: impl Into<String>, value: impl Into<String>);
    pub fn prefix(&self) -> Option<&str>;
    pub fn checksum(&self) -> Option<&[u8]>;                               // stored trailer of a read PBO
}
pub struct SelectedEntry { /* name, data */ }
impl SelectedEntry { pub fn name(&self) -> &str; pub fn data(&self) -> Result<&[u8], &Error>; }
impl PboSelection {
    pub fn names(&self) -> &[String];
    pub fn entries(&self) -> &[SelectedEntry];
    pub fn header_extension(&self, key: &str) -> Option<&str>;
    pub fn header_extensions(&self) -> impl Iterator<Item = (&str, &str)>;
}
// sign.rs
impl BIPrivateKey { pub fn name(&self) -> &str; }
impl BIPublicKey { pub fn name(&self) -> &str; }
pub fn signature_path(pbo_path: &Path, key_name: &str) -> Result<PathBuf, Error>; // today's private default_signature_path, made public
```

  `BIPrivateKey::sign` and `BIPublicKey::verify`: when `pbo.checksum()` is `None`, compute the SHA-1 the written PBO
  would have (write into a hashing `Write` adapter, no temp file); when it is `Some`, use it unchanged.
  `type SelectedEntry = (…)` is replaced by the struct (API break, fine in 0.x; update `src/api/mod.rs`).

- [ ] **Step 1: Write the failing tests** (`pbo_api.rs`)

```rust
#[test] fn signing_a_fresh_pbo_matches_signing_it_after_a_roundtrip() {
    // pbo = PBO::from_directory(golden mod copy, &PackOptions::pack(), None, &mut Warnings::new())
    // key = BIPrivateKey::read(tests/fixtures/bi/sign/armk.biprivatekey)
    // a = key.sign(&pbo, V3) bytes; b = key.sign(&PBO::read(&pbo.to_cursor())?, V3) bytes
    // assert_eq!(a, b); key.to_public_key()?.verify(&reread, &sig_a) is Ok
}
#[test] fn signing_a_read_pbo_uses_its_stored_checksum() {
    // tests/fixtures/bi/sign/testsig.pbo: sign with armk → bytes equal tests/fixtures/bi/sign/<the BI .bisign for testsig>
}
#[test] fn accessors_expose_entries_and_extensions_in_order() {
    // PBO::new(); insert_entry("b.txt"), insert_entry("a\\c.txt"); set_header_extension("prefix","p"), ("x","1")
    // write + read: entry_names == ["a\\c.txt","b.txt"] (write sorts case-insensitively, check today's rule),
    // header_extensions == [("prefix","p"),("x","1")], prefix() == Some("p"), checksum().is_some()
}
#[test] fn insert_entry_rejects_unsafe_names() {
    for bad in ["", "..\\x", "/abs", "C:\\x", "a\\..\\..\\x"] { assert!(PBO::new().insert_entry(bad, vec![]).is_err()); }
}
#[test] fn selection_gives_requested_entries() { /* read_entries on the golden PBO for ["data\\readme.txt"] → entries()[0].data() == file bytes */ }
```

- [ ] **Step 2: Run** → FAIL (methods missing; fresh-PBO sign errors with "PBO has no checksum").
- [ ] **Step 3: Implement**; update all users.
- [ ] **Step 4: Run** `cargo test --workspace --locked` → PASS; snapshots unchanged; `tests/bi_golden.rs` and the
  BI signature comparison in `tests/pbo.rs` unchanged and passing; `make fuzz` builds.
- [ ] **Step 5: `make check`, commit** — `feat: PBO accessors and signing of unwritten PBOs in dayzmake-core`

---

### Task 9: PAA API: `PaaFormat`, private fields

**Files:**
- Modify: `crates/dayzmake-core/src/paa.rs`, `src/commands/paa.rs`, `src/mcp.rs`, tests and fuzz target that touch
  `Paa` fields
- Test: `crates/dayzmake-core/tests/paa_api.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaaFormat { Dxt1, Dxt3, Dxt5, Rgba }
impl std::str::FromStr for PaaFormat { type Err = std::io::Error; } // case-insensitive "dxt1|dxt3|dxt5|rgba"; error text exactly today's "Unsupported PAA compression type for creation: {s}"
impl Paa {
    pub fn from_dynamic_image(img: &DynamicImage, format: PaaFormat) -> Result<Paa, Error>; // the unused `_compress` parameter is gone
    pub fn width(&self) -> u16; pub fn height(&self) -> u16;
    pub fn compression(&self) -> &str;        // as read from the file, e.g. "DXT5"
    pub fn mipmap_count(&self) -> usize;
    pub fn tagg(&self, name: &str) -> Option<&[u8]>;
}
pub use image;                                // in lib.rs: consumers use the same image version as the API
```

  The CLI parses `-t` with `PaaFormat::from_str` and still accepts `-c` (it did nothing before and still does
  nothing; keep the flag so scripts do not break).

- [ ] **Step 1: Failing tests**

```rust
#[test] fn format_parses_case_insensitively() { assert_eq!("DxT5".parse::<PaaFormat>().unwrap(), PaaFormat::Dxt5); }
#[test] fn unknown_format_has_the_cli_error_text() { assert!("bogus".parse::<PaaFormat>().unwrap_err().to_string().contains("Unsupported PAA compression type for creation: bogus")); }
#[test] fn dxt1_and_dxt5_roundtrip_dimensions() {
    // 64×64 gradient → from_dynamic_image(Dxt1/Dxt5) → write → read: width()==64, height()==64, compression()=="DXT1"/"DXT5", mipmap_count() >= 1
}
#[test] fn conversion_matches_the_cli_bytes() {
    // same gradient as golden_cli.rs → bytes equal the `img2paa_dxt5` binary snapshot file (read it from tests/snapshots)
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test --workspace --locked` → PASS,
  snapshots unchanged (`img2paa_bad_type` stderr identical).
- [ ] **Step 5: `make check`, commit** — `feat: PaaFormat and accessors for Paa in dayzmake-core`

---

### Task 9b: Replace the GPL-2.0-only `lzo1x` crate

`lzo1x 0.2.2` is licensed `GPL-2.0` (only; its LICENSE is plain GPLv2, no "or later" anywhere). That is
incompatible with AGPL-3.0 consumers such as Náves, and `Paa::from_dynamic_image` LZO-compresses every DXT mipmap
wider than 128 px, so a consumer cannot avoid it.

**Files:**
- Modify: `crates/dayzmake-core/Cargo.toml`, `crates/dayzmake-core/src/paa.rs`, `deny.toml` (the `GPL-2.0` entry
  in `allow` goes once nothing needs it), `Cargo.lock`, `fuzz/Cargo.lock`
- Test: `crates/dayzmake-core/tests/paa_api.rs`, the existing LZO tests in `tests/paa.rs`

**Interfaces:**
- No public API change. Internal: `fn lzo_compress(data: &[u8]) -> Vec<u8>` and
  `fn lzo_decompress(data: &[u8], out_len: usize) -> Result<Vec<u8>, Error>` in `paa.rs` wrap the new crate, so the
  crate is used in one place.

- [ ] **Step 1: Pick the crate.** Candidates: `lzokay-native` (MIT), `lzallright`, `rust-lzo1x`, `lzo` (decoder
  only), `decode-lzo` (decoder only). Review per AGENTS.md (licence on the `deny.toml` allow-list and compatible with
  GPL-2.0-or-later *and* AGPL-3.0, i.e. permissive or GPL-2.0-or-later; safe Rust or justified `unsafe`; no panics on
  hostile input in the decompressor; maintenance). A compressor and a decompressor may come from different crates.
  If none passes, stop and report BLOCKED with the review.
- [ ] **Step 2: Failing tests** (in `paa_api.rs`)

```rust
#[test] fn lzo_compressed_mipmaps_decompress_to_the_dxt_data() {
    // 256×256 gradient → from_dynamic_image(Dxt5) → write → read → to_dynamic_image():
    // pixels equal those of the same image converted to Dxt5 at 128×128 scale-free path… simpler:
    // compare to_dynamic_image() of the new PAA with the decoded `img2paa_dxt5`-style result of the old build,
    // stored as a binary snapshot `lzo_roundtrip.rgba` recorded BEFORE swapping the crate
}
#[test] fn hostile_lzo_streams_are_errors() { /* the existing paa_short_mipmap / huge fixtures + 64 random byte strings (fixed seed) as LZO mipmaps → Err, no panic */ }
```

  Record `lzo_roundtrip.rgba` with the old crate first (commit it in this task's first commit, `test:`), then swap.
- [ ] **Step 3: Swap the crate.** Keep the rule "use the compressed data only if it is shorter" and the end-marker
  rule (`11 00 00` last, nothing after it; `an_lzo_stream_ends_with_its_marker_and_nothing_after_it` must pass).
- [ ] **Step 4: Verify.** `cargo test --workspace --locked`: PASS. Snapshots that may change, and only these: the
  PAA byte snapshots of images wider than 128 px (the Task 1 PNGs are 64×64, so normally none). Any other change is
  a regression. `cargo deny check licenses` shows no `GPL-2.0`-only crate:
  `cargo tree -p dayzmake-core -e normal --format '{p} {l}' | grep -E 'GPL-2.0($| )'` → no output.
- [ ] **Step 5: In-game note.** The new compressor's output differs from the old one byte-wise; the decompressed
  mipmaps are identical. Add to the report that a PAA > 128 px made with the new build should be checked in the game
  (Task 13 does it).
- [ ] **Step 6: `make check`, commit** — `fix: replace the GPL-2.0-only lzo1x crate` (body: crate review, why).

---

### Task 10: Config read access and `#[non_exhaustive]` model types

**Files:**
- Modify: `crates/dayzmake-core/src/config.rs`, `crates/dayzmake-core/src/p3d.rs`
- Test: `crates/dayzmake-core/tests/config_api.rs`, doctest in `p3d.rs`

**Interfaces:**
- Produces:

```rust
impl Config { pub fn root(&self) -> &ConfigClass; }
impl ConfigClass {
    pub fn parent(&self) -> Option<&str>;          // None when there is no parent
    pub fn is_external(&self) -> bool;
    pub fn is_deletion(&self) -> bool;
    pub fn entries(&self) -> impl Iterator<Item = (&str, &ConfigEntry)>;   // file order; empty for external/deletion
    pub fn get(&self, name: &str) -> Option<&ConfigEntry>;                 // ASCII case-insensitive, like the game
}
impl ConfigArray { pub fn elements(&self) -> &[ConfigArrayElement]; pub fn is_expansion(&self) -> bool; }
```

  `#[non_exhaustive]` on `ConfigEntry`, `ConfigArrayElement`, `p3d::{P3D, LOD, Face, Vertex, Point, Tagg}`.

- [ ] **Step 1: Failing tests**

```rust
#[test] fn read_values_from_a_parsed_config() {
    // Config::from_string of the golden config.cpp (with include folder) →
    // root().get("cfgpatches") is ClassEntry; its first class has get("units") ArrayEntry with elements()
    // equal to the strings in the fixture; a nested class reports parent() == Some("<base>")
}
#[test] fn read_values_from_a_rapified_config() { /* same assertions on Config::read_rapified(to_cursor()) */ }
```

  Doctest on `P3D`:

```rust
/// ```compile_fail
/// let _ = dayzmake_core::p3d::P3D { version: 0, lods: Vec::new() };
/// ```
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, snapshots unchanged.
- [ ] **Step 5: `make check`, commit** — `feat: read access to parsed configs; non_exhaustive model types`

---

### Task 11: Cargo features and packaging of `dayzmake-core`

**Files:**
- Modify: `crates/dayzmake-core/Cargo.toml`, `crates/dayzmake-core/src/{lib,paa,sign}.rs`, `Makefile`,
  `.github/workflows/ci.yml`, `deny.toml` only if `cargo deny` needs it
- Create: `crates/dayzmake-core/README.md`

**Interfaces:**
- Produces: features `default = ["sign", "image"]`, `sign = ["dep:openssl"]` (gates module `sign`),
  `image = ["dep:image", …]` (gates `Paa::from_dynamic_image`, `Paa::to_dynamic_image`, `pub use image`; put
  `texpresso` behind it only if `Paa::read`/`write` do not need it). Gated items carry
  `#[cfg_attr(docsrs, doc(cfg(feature = "…")))]` and `[package.metadata.docs.rs] all-features = true`.
  Package metadata: `readme = "README.md"`, `keywords = ["dayz", "pbo", "paa", "modding", "bohemia"]`,
  `categories = ["parser-implementations", "game-development"]`,
  `include = ["src/**/*", "README.md", "LICENSE", "Cargo.toml"]`.
- Makefile: `features` target running
  `cargo clippy -p dayzmake-core --no-default-features --lib --locked -- -D warnings`,
  the same with `--features sign`, and with `--features image`; `package-check` target running
  `cargo package -p dayzmake-core --locked --allow-dirty` (`--allow-dirty` only in the Makefile for local runs; CI
  runs without it). Both are added to `check`; CI lint job gets the same steps.

- [ ] **Step 1: Failing check:** add the `features` target first and run `make features` → FAIL (no features
  defined, `--features sign` unknown).
- [ ] **Step 2: Implement** the features and metadata. The core README: what the crate is, the licence note
  (GPL-2.0-or-later; AGPL/GPLv3 projects may link it), features, a short example (same as the lib.rs docs of
  Task 12, `ignore`d there to avoid two copies: link to the docs instead).
- [ ] **Step 3: crates.io readiness for both crates**
  - Root `Cargo.toml`: `include = ["src/**/*", "README.md", "LICENSE", "Cargo.toml"]` (keeps `tests/fixtures`
    with the throwaway key, `testharness/`, `docs/`, `fuzz/` out of the package), `keywords`, `categories`
    (`command-line-utilities`, `game-development`), `readme`.
  - `package-check` becomes `cargo package --workspace --locked` (packages both crates; the CLI is verified
    against the packaged core). CI runs it without `--allow-dirty`.
  - Badges at the top of `README.md` (for both crates) and `crates/dayzmake-core/README.md` (for the core):
    `[![crates.io](https://img.shields.io/crates/v/dayzmake-core.svg)](https://crates.io/crates/dayzmake-core)`,
    `[![docs.rs](https://docs.rs/dayzmake-core/badge.svg)](https://docs.rs/dayzmake-core)`, and the same
    crates.io badge for `dayzmake`. Keep the existing badges.
  - `AGENTS.md`, new section *Releasing*: bump versions (core follows semver on its own; a breaking core change
    bumps the `version` in the CLI's dependency too), `make check`, tag, then `cargo publish -p dayzmake-core`
    **before** `cargo publish -p dayzmake`. Publishing is done by the maintainer, never by an agent.
- [ ] **Step 4: Run** `make features package-check` → PASS; `cargo package --list -p dayzmake-core` and
  `-p dayzmake` show no `tests/fixtures`, no `secrets`, no key files, no `testharness`. `make check` → PASS.
- [ ] **Step 5: Commit** — `build: features, package metadata and crates.io readiness`

---

### Task 12: Consumer test, crate docs, README, AGENTS.md

**Files:**
- Create: `crates/dayzmake-core/tests/consumer.rs`
- Modify: `crates/dayzmake-core/src/lib.rs` (crate docs), `README.md` (new section *Using dayzmake-core as a
  library*; no CLI change, so `USAGE` stays), `AGENTS.md` (library rules: no printing, `WarningSink` for warnings,
  options structs for new parameters, `#[non_exhaustive]` on public data types, semver of `dayzmake-core`)

**Interfaces:**
- Consumes: Tasks 5–10 APIs only.

- [ ] **Step 1: Write the consumer test** (mirrors Náves's four uses, public API only)

```rust
#[test] fn naves_workflow_in_parallel() {
    // 1. keygen: BIPrivateKey::generate(1024, "naves_test"), write .biprivatekey and to_public_key() .bikey into tmp/secrets
    // 2. 16 threads, each converts its own 64×64 image to PAA: 8 × Dxt1 ("s_*"), 8 × Dxt5 ("m_*"),
    //    each with its own Warnings; write into tmp/stage/data/; assert every read-back PAA has the right compression()
    // 3. pack + sign: PBO::from_directory(stage, &PackOptions::pack(), None, &mut w), write to tmp/addons/naves_test.pbo,
    //    key.sign(&pbo, V3) → signature_path(...); verify with the public key against PBO::read of the written file → Ok
    // 4. read selected entries: PBO::read_entries on the written PBO for two of the PAA names → data equals the files
}
```

- [ ] **Step 2: Run** `cargo test -p dayzmake-core --test consumer --locked` → PASS (all APIs exist now; if
  anything is missing or awkward, fix the API in its owning task's style and note it in the commit).
- [ ] **Step 3: Crate docs** in `lib.rs`: an overview of the modules, the warning and binarize model, and the
  workflow above as a doctest that runs (keygen in a `tempfile` dir, one PAA, pack, sign, verify).
  Add `tempfile` as a dev-dependency only (it is one already).
- [ ] **Step 4:** `make check` → PASS.
- [ ] **Step 5: Commit** — `docs: dayzmake-core usage, consumer test and library rules`

---

### Task 13: Differential run against the v2.0.0 release

Final proof on real data that the CLI did not change. Manual, not in CI.

**Files:**
- Create: `testharness/compare-release.sh`, section in `testharness/README.md`

**Interfaces:**
- `testharness/compare-release.sh <old-dayzmake> <new-dayzmake> <dayz-dir>`: work dir from
  `mktemp -d /var/tmp/dayzmake-compare.XXXXXX`; for each `<dayz-dir>/addons/*.pbo` (read-only): `inspect`,
  `unpack` to `old/` and `new/`, `derapify` of every `config.bin` found, `inspect` of the first 50 `.p3d` found
  (sorted); plus every command of `tests/golden_cli.rs` on `tests/fixtures/golden/mod`. Compare outputs and exit
  codes with `diff -r` and `cmp`; print a summary (`N identical, M different`) and the list of differences. Never
  writes inside `<dayz-dir>`; removes the work dir unless `KEEP=1`.

- [ ] **Step 1: Build the old binary**

```bash
OLD=$(mktemp -d /var/tmp/dayzmake-v2.XXXXXX)
git worktree add "$OLD/src" v2.0.0
CARGO_TARGET_DIR="$OLD/target" cargo build --release --locked --manifest-path "$OLD/src/Cargo.toml"
make -C . && cargo build --release --locked   # new binary in $CARGO_TARGET_DIR/release/dayzmake
```

- [ ] **Step 2: Write the script**, then run it against the DayZ server install
  (`~/.steam/steam/steamapps/common/DayZServer` or the path the user names; ask if neither exists).
  Expected: only these differences:
  - PBOs with more than one non-prefix header extension: order in `inspect` output and in unpacked `$PBOPREFIX$`
    (Task 3; check the new order is the file order)
  - suppression summary lines on stderr (Task 5)
  - LZO-compressed PAA mipmap bytes (Task 9b); decompressed pixels must be identical. Convert one PNG wider
    than 128 px with the new binary and ask the user to load it in the game (a texture on a test item); record
    the answer.
  Anything else is a regression: stop, write a failing test in the owning task's test file, fix, rerun.
- [ ] **Step 3: Clean up:** `git worktree remove "$OLD/src"`, `rm -rf "$OLD"`.
- [ ] **Step 4: `make check`, commit** — `test: differential comparison against a released dayzmake`
  (the commit message lists the run's summary: number of PBOs/configs/models compared, differences found and why
  each is expected).
