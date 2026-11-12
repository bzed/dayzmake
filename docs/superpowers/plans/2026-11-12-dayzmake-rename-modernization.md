# dayzmake Rename & Modernization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the armake2 fork to **dayzmake** everywhere (binary, crate, docs, CI, test harness), pin the latest stable Rust toolchain with edition 2024, and integrate naves' build/lint/security standard (clippy pedantic + security lints, cargo deny, CodeQL, Makefile) — fixing the code until it passes.

**Architecture:** One task per layer: rename first (so all later diffs are in final naming), then toolchain, then quality infrastructure, then lint fixes module-by-module so each commit leaves the tree buildable and tested. No behavior changes beyond the rename itself (usage text, error strings, temp dir names, env var names); all existing tests must keep passing unchanged in what they assert about file formats.

**Tech Stack:** Rust 1.99 (stable, pinned), edition 2024, docopt CLI (kept), cargo clippy (naves lint set), cargo deny, CodeQL, GitHub Actions.

**Spec:** the user request (2026-11-12) plus `../naves/AGENTS.md` ("Safe Rust", "Dependencies", "Before pushing" sections) and `../naves/{Makefile,clippy.toml,deny.toml,rust-toolchain.toml,.github/workflows/{ci,codeql}.yml}` as the reference implementations.

## Global Constraints

- New name is **dayzmake** in `kebab-case` for the binary/crate, `dayzmake` in prose. Repo: `github.com/bzed/dayzmake` (remote already renamed).
- Env vars rename too: `ARMAKE2_BIN` → `DAYZMAKE_BIN`, `ARMAKE2_PROTON_TESTS` → `DAYZMAKE_PROTON_TESTS`, `ARMAKE2_P3D_CORPUS` → `DAYZMAKE_P3D_CORPUS`.
- Test mod renames: `armake2_test` → `dayzmake_test` (folder, CfgPatches class, `$PREFIX$` content, marker string, `-servermod=@dayzmake_test`).
- Toolchain: channel `1.99` in `rust-toolchain.toml` (components `rustfmt`, `clippy`); `edition = "2024"`, `rust-version = "1.99"` in `Cargo.toml`.
- Lint set (in `[lints]` of `Cargo.toml`, copied from naves): `rust`: `unsafe_code = "forbid"`, `missing_docs = "warn"`; `clippy`: `all` + `pedantic` warn, `unwrap_used`/`expect_used`/`panic`/`indexing_slicing`/`print_stdout`/`print_stderr`/`allow_attributes_without_reason` warn, `cast_possible_truncation`/`cast_sign_loss`/`cast_possible_wrap`/`float_cmp` deny. CI and `make lint` run with `-D warnings`.
- `clippy.toml`: `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`, `allow-indexing-slicing-in-tests` all `true`; `doc-valid-idents = ["DayZ", "BI", "PBO", "PAA", "MLOD", "ODOL", "BMTR", "RTM", "RVMAT", "OpenSSL", "Proton", "GitHub", ".."]`.
- `[profile.release] overflow-checks = true`.
- `deny.toml`: naves' structure; license allow-list adapted to GPL-2.0-or-later compatibility (keeps `MPL-2.0`, adds `GPL-2.0`; lzo1x is GPL-2.0 — needs an explicit allow entry).
- `#[expect(lint, reason = "...")]` instead of `#[allow]`; never silence a lint crate-wide. CLI `cmd_*` functions that print get `#[expect(clippy::print_stdout, reason = "CLI command output")]`.
- Historical documents stay as-is: `docs/superpowers/plans/*.md` are records of past work and keep the armake2 name.
- `appveyor.yml` is deleted (dead upstream legacy; CI is GitHub Actions).
- Every task ends with `cargo test` green and a commit using the repo's `component: description` style.

## Review Focus

1. **Byte-identical outputs survive the lint fixes** — cast hardening (`as` → `TryFrom`/checked) must not change PBO/P3D/PAA/signature bytes. Pinned by `tests/bi_golden.rs`, `tests/p3d.rs` round-trip, `tests/pbo.rs` sign/verify against BI fixtures; those tests must not be edited to accommodate code changes.
2. **Piped usage** (`dayzmake rapify < in > out`) still works after the run.rs rename — `tests/cli.rs` covers it; keep it.
3. **Untrusted input does not panic** after `unwrap_used`/`indexing_slicing` fixes — garbage inputs to `P3D::read`/config parser must error, not abort; `p3d.rs` unit tests (`garbage_is_an_error_not_a_panic`) pin this; add equivalent garbage-input tests for config and PBO parsing in the owning tasks.
4. **`--help`/`--version`/error text say dayzmake** and exit codes are unchanged — verify by running the built binary in Task 1 and Task 9.
5. **The test harness still builds and signs its mod** with the renamed binary and `@dayzmake_test` — `testharness/run.sh --no-server` must pass (PBO/signature/verify path); it is the rename's end-to-end check.

---

### Task 1: Rename everything to dayzmake

**Files:**
- Modify: `Cargo.toml` (name, lib name, description "DayZ modding tools", homepage/repository → `github.com/bzed/dayzmake`), `Cargo.lock` (regenerate via `cargo update --workspace`-less build)
- Modify: `src/main.rs`, `src/run.rs` (USAGE string), `src/eval.rs` (error text), `src/binarize.rs` (temp dir prefixes `armake2-binarize-`, sandbox name `armake_`), `src/pbo.rs`, `src/preprocess.rs`, `src/config.rs` (doc comments `use armake2::`)
- Modify: `benches/config.rs`, `benches/preprocess.rs`, all `tests/*.rs` (`CARGO_BIN_EXE_dayzmake`, `use dayzmake::`, helper fns `armake2()`→`dayzmake()`, env var names)
- Modify: `testharness/run.sh`, `testharness/README.md`; rename `testharness/mod/addons/armake2_test/` → `dayzmake_test/` (incl. `$PREFIX$` content, `config.cpp` classes/defines, script filename, marker)
- Modify: `README.md` (title, prose, usage blocks; badges: drop dead travis/appveyor/crates.io-armake2 badges, add GitHub Actions CI badge `https://github.com/bzed/dayzmake/actions/workflows/ci.yml`, CodeQL badge, `GPL-2.0-or-later` license badge), `tests/fixtures/bi/README.md`
- Modify: `.github/workflows/ci.yml` + `release.yml` (artifact/binary names `dayzmake`, `dayzmake-…-debian-trixie-x86_64`), `Dockerfile` (`/usr/src/dayzmake`), `build.sh` (image/container/binary names)
- Delete: `appveyor.yml`

**Interfaces:**
- Produces: binary `dayzmake`, lib crate `dayzmake`, env vars `DAYZMAKE_{BIN,PROTON_TESTS,P3D_CORPUS}`, test mod `dayzmake_test`. Later tasks rely on exactly these names.

- [x] **Step 1: Apply the rename** with `git mv` for the test mod folder, then mechanical replacements (case-aware: `armake2`→`dayzmake`, `ARMAKE2`/`ARMAKE`→`DAYZMAKE`, `KoffeinFlummi` repo URLs→`bzed/dayzmake`). Keep the "Successor to armake" history paragraph but reframe: fork of armake2, diverged for DayZ.
- [x] **Step 2: Rebuild lockfile** (`cargo build` regenerates `Cargo.lock` package name).
- [x] **Step 3: Verify**: `cargo build && cargo test` green; `./target/debug/dayzmake --help | head -3` shows `dayzmake`; `--version` prints `v0.3.0`; `grep -ri armake --exclude-dir={target,.git} .` returns only `docs/superpowers/plans/` hits and the upstream-fork note in README.
- [x] **Step 4: Commit** `rename: armake2 → dayzmake`

### Task 2: Pin toolchain, edition 2024, refresh dependencies

**Files:**
- Create: `rust-toolchain.toml` (channel `1.99`, components `rustfmt`, `clippy`)
- Modify: `Cargo.toml` (`edition = "2024"`, `rust-version = "1.99"`), `Cargo.lock` (`cargo update`)

**Interfaces:** none new.

- [x] **Step 1: Write `rust-toolchain.toml`, set edition fields.**
- [x] **Step 2: `cargo fix --edition --allow-dirty --all-targets`** then flip `edition = "2021"` → `"2024"`; fix remaining 2024 hardening errors manually (reserved words, `static mut`—none expected; no `unsafe` in src).
- [x] **Step 3: `cargo update`** (semver-compatible refresh) so RustSec advisories in Task 3 have a chance to pass; check `cargo tree --duplicates` stays sane.
- [x] **Step 4: Verify**: `cargo build --locked && cargo test --locked` green on 1.99.
- [x] **Step 5: Commit** `build: pin Rust 1.99, edition 2024, refresh dependencies`

### Task 3: Quality infrastructure (naves-style)

**Files:**
- Create: `clippy.toml`, `deny.toml`, `Makefile`, `.github/workflows/codeql.yml`
- Modify: `Cargo.toml` (`[lints]` per Global Constraints, `[profile.release] overflow-checks = true`), `.github/workflows/ci.yml` (add `lint` job running fmt-check/clippy `-D warnings`/doc/deny — expect it red until Tasks 5–8 land, so gate it with `continue-on-error: false` only at the end of Task 8; simplest: add the lint job in this task but let it fail is NOT acceptable — instead add the lint job commented-in only after Task 8. Decision: add the `lint` job to ci.yml in Task 8's final step.)

Correction (binding): `Makefile` and `codeql.yml` land here; the `lint` job in `ci.yml` lands with Task 8. `Makefile` targets: `fmt`, `fmt-check`, `lint`, `doc`, `deny`, `test`, `check` (fmt-check + lint + doc + deny + test), mirroring naves minus coverage/fuzz/deb/docs targets this repo doesn't have.

**Interfaces:** Produces `make check` as the pre-push gate later tasks must satisfy.

- [ ] **Step 1: Write `clippy.toml`, `deny.toml` (GPL-2.0 allow list incl. lzo1x GPL-2.0 exception), `Makefile`, `codeql.yml` (languages `rust`+`actions`, build-mode `none`, weekly + push/PR on `master`).**
- [ ] **Step 2: Add `[lints]` and release `overflow-checks = true` to `Cargo.toml`.**
- [ ] **Step 3: Run `cargo fmt --all` once** (tree-wide mechanical reformat, its own commit).
- [ ] **Step 4: Verify**: `cargo deny check` passes (fix advisories via `cargo update -p` if flagged); `make fmt-check` and `make deny` pass. `cargo clippy` still warns (fixes come next) — acceptable here.
- [ ] **Step 5: Commit** `build: naves-style lint/security infrastructure` + `fmt: cargo fmt`

### Task 4: Fix clippy::all warnings

**Files:** Modify: whichever files the 88 findings live in (all of `src/`, `benches/`, `tests/`).

- [ ] **Step 1: `cargo clippy --fix --all-targets --allow-dirty`** then fix the rest by hand (notably: regex compiled in a loop → `LazyLock`, `.bytes()` on in-memory data, `&PathBuf`→`&Path` params, needless `vec!`).
- [ ] **Step 2: Verify**: `cargo clippy --all-targets -- -D clippy::all` clean; `cargo test` green; byte-identity tests untouched.
- [ ] **Step 3: Commit** `fix: clippy::all warnings`

### Task 5: Full lint set — error, io, eval, run, main, lib

**Files:** Modify: `src/error.rs`, `src/io.rs`, `src/eval.rs`, `src/run.rs`, `src/main.rs`, `src/lib.rs`.

**Interfaces:** Produces: error-handling helpers the other modules already use (`error!`, `warning`, `print_error`); no signature changes unless a lint forces it.

- [ ] **Step 1: Fix per the full set**: `once_cell` → `std::sync::LazyLock` (drops a dependency); printing in `error.rs`/`run.rs` gets `#[expect(clippy::print_stdout, reason = "CLI output")]`; `stdin().read_to_end().unwrap()` in `run.rs` becomes an error; `unreachable!()` in `run_command` becomes an `error!`; missing rustdoc on pub items; `#[allow]` → `#[expect]`.
- [ ] **Step 2: Verify**: `cargo clippy --all-targets -- -D warnings 2>&1 | grep -E "^error|error.rs|io.rs|eval.rs|run.rs|main.rs|lib.rs"` shows no findings for these files (other modules still warn); `cargo test` green.
- [ ] **Step 3: Commit** `lint: error, io, eval, run to the full standard`

### Task 6: Full lint set — config, preprocess

**Files:** Modify: `src/config.rs`, `src/preprocess.rs`.

- [ ] **Step 1: Fix**: unwraps on untrusted input → errors; slice indexing → `get(…).ok_or_else(|| error!(…))`; pedantic (doc sections, format strings, pass-by-value, `String` vs `&str`); float equality (`n.fract() == 0.0`, `y == 0.0`) → `matches!(x.classify(), std::num::FpCategory::Zero)`; missing rustdoc.
- [ ] **Step 2: Add garbage-input tests**: truncated rapified config bytes and a truncated preprocessed file must return errors, not panic (mirror `garbage_is_an_error_not_a_panic`).
- [ ] **Step 3: Verify**: no clippy findings for these two files; `cargo test` green (incl. new tests).
- [ ] **Step 4: Commit** `lint: config, preprocess to the full standard`

### Task 7: Full lint set — p3d, paa

**Files:** Modify: `src/p3d.rs`, `src/paa.rs`.

- [ ] **Step 1: Fix**: the ~45 cast findings live mostly here (`as u32`/`as f32`/`as usize`) → `TryFrom` + `error!` context or checked arithmetic; indexing → `get`; unwraps → errors; missing rustdoc.
- [ ] **Step 2: Verify**: no clippy findings for these files; `cargo test` green; `tests/p3d.rs` round-trip and PAA golden tests unchanged and passing.
- [ ] **Step 3: Commit** `lint: p3d, paa to the full standard`

### Task 8: Full lint set — pbo, binarize, sign; enable CI lint job

**Files:** Modify: `src/pbo.rs`, `src/binarize.rs`, `src/sign.rs`, `.github/workflows/ci.yml`.

- [ ] **Step 1: Fix** the three modules (sign.rs has the most unwraps; most are in its test module which `clippy.toml` exempts — the non-test ones need error returns). Add a garbage-input test for PBO parsing (truncated header/entries error out instead of panicking), pinning Review Focus #3.
- [ ] **Step 2: Verify**: `cargo clippy --all-targets --locked -- -D warnings` fully clean; `cargo doc --no-deps` with `RUSTDOCFLAGS="-D warnings"` clean; `cargo test` green.
- [ ] **Step 3: Add the `lint` job to ci.yml** (fmt-check, clippy `-D warnings`, doc, deny on ubuntu-latest) and the `windows` job stays test-only.
- [ ] **Step 4: Commit** `lint: pbo, binarize, sign to the full standard; ci: lint job`

### Task 9: Final verification and README consistency

- [ ] **Step 1: `make check`** (fmt-check, lint, doc, deny, test) passes.
- [ ] **Step 2: `testharness/run.sh --no-server`** passes with the renamed binary and `@dayzmake_test` mod.
- [ ] **Step 3: README usage block matches `dayzmake --help` output exactly** (diff the two); badges resolve (CI, CodeQL, license).
- [ ] **Step 4: `grep -ri armake` outside `docs/superpowers/plans/` is empty**; push or report.
