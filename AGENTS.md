# AGENTS.md

Rules for anyone (human or agent) changing dayzmake. dayzmake is a DayZ modding CLI (build, sign, verify,
pack, unpack, binarize, derapify, PAA conversion). It is a fork of armake2 by KoffeinFlummi, licensed
**GPL-2.0-or-later**; keep the licence and the original copyright holders (`Cargo.toml`, `LICENSE`). It reads
PBO, config, P3D, PAA and key/signature files from mods the user did not write, so every reader must treat its
input as hostile.

## Repository layout

| Path | Content |
|---|---|
| `crates/dayzmake-core/` | library crate `dayzmake-core`: the file formats and their plumbing |
| `crates/dayzmake-core/src/` | `pbo`, `config`, `p3d`, `paa`, `sign`, `io` (readers and writers), `preprocess`, `eval`, `binarize`, `error`, `warning` |
| `crates/dayzmake-core/tests/` | library tests (config, preprocessor, BI golden files, fuzz seeds of the format readers) |
| `crates/dayzmake-core/benches/` | `criterion` benchmarks (`preprocess`, `config`) |
| `src/*.rs` | root crate `dayzmake`: the CLI (`run.rs`, `main.rs`), the MCP server and the script API index (`api/`) |
| `tests/` | binary-driven tests (CLI, MCP, snapshots) and tests of the root crate; `tests/fixtures/` is shared with core |
| `tests/fixtures/bi/` | reference output of BI's tools on our own inputs (see its `README.md`) |
| `testharness/` | end-to-end test against a real DayZ server (manual, not run by CI) |
| `fuzz/` | `cargo-fuzz` crate: targets for the untrusted-input readers, with seed corpora |
| `docs/book/` | the HTML documentation (mdBook): guides, command reference, theme; published to GitHub Pages by `.github/workflows/pages.yml` |
| `docs/superpowers/` | design notes and plans |

The repository is a Cargo workspace (root crate `dayzmake` plus `crates/dayzmake-core`); lints are set once in
`[workspace.lints]`.

Toolchain: Rust **1.99**, edition 2024, pinned in `rust-toolchain.toml`. `Cargo.lock` is committed and CI builds
with `--locked`. Fuzzing and Miri need `nightly` (see below).

## Project ground rules

- **No game data in the repo.** Test fixtures are synthetic, or BI tool output made from our own inputs
  (`tests/fixtures/bi/README.md`). Game files are read from the user's install at run time.
- **Clean-room:** no code from BI tools. Formats come from public documentation and from observing game files
  and tool output on our own inputs. armake2 code stays under its own licence.
- Using the DayZ client, server and DayZ Tools for development and manual testing is allowed. Prefer your own
  folders (a server tree, a mod folder) over changes inside an install, never edit or delete game files, remove
  what you linked when you are done. Local DayZ test servers never use the default ports. The automated tests
  only read what the repo provides.
- Temporary and build folders go under `/tmp` or `/var/tmp` (`tempfile` crate or `mktemp -d`), never under
  `$HOME`. The `Makefile` sets `TMPDIR` and `CARGO_TARGET_DIR` accordingly.
- Linux first; everything except clearly marked platform code (`winreg`, Proton/Wine paths) must also build on
  Windows.
- **Reuse before you write:** check for an existing crate first and review it (see *Dependencies*).
- The licence stays GPL-2.0-or-later. Do not add code or dependencies under an incompatible licence.

## Tests

- **Test first.** A new feature or bug fix comes with a test that fails without it (red, then green).
- Test behaviour through the public API: integration tests in `tests/`. Unit tests in `#[cfg(test)] mod tests`
  are for private helpers that hold real logic.
- Tests are **deterministic**: no `sleep` to wait for state, fixed seeds for random input, no network access.
- Tests write only under a `tempfile::TempDir`. They never touch `$HOME`, the user's config, a Steam directory or
  a running server. CLI tests set `HOME` and the `XDG_*` variables to temp dirs.
- Code that calls external tools (`binarize.exe`, Proton, DayZ) is tested in CI against fakes. Tests that need the
  real tools are `#[ignore = "needs <tool>"]` and run explicitly; they do not replace the faked tests.
- Parsers for untrusted input (PBO, config, P3D, PAA, keys, signatures) have a fuzz target (`make fuzz`).
  A crash found by fuzzing gets a regression test in `tests/` with the minimised input, in the same commit as
  the fix. The fuzz corpus seeds also run as ordinary tests in `make test` (`tests/fuzz_seeds.rs` and `crates/dayzmake-core/tests/fuzz_seeds.rs`).

## Coverage

- Total line coverage stays **at or above 70 %** (`COVERAGE_MIN` in the `Makefile`, `cargo llvm-cov`). A change
  must not lower it; raise the floor when coverage grows.
- CI uploads the report to **Codecov** (`codecov.yml`: `project` `threshold: 0%`, `patch` `threshold: 5%`).
- Never lower the threshold, and never exclude code (`--ignore-filename-regex`, `#[coverage(off)]`) to reach it.

## Safe Rust

The lints are configured once, in `[workspace.lints]` in the root `Cargo.toml`; `clippy.toml` holds the test exceptions.

**Lints**
- Strict by design: `cargo clippy` runs with `-D warnings`; `clippy::all` and `clippy::pedantic` are enabled.
  Don't relax the lint set to get a change through.
- Silence a lint locally with `#[expect(lint, reason = "...")]`, not `#[allow]`
  (`clippy::allow_attributes_without_reason` is on). Never silence a lint crate-wide to make CI pass.

**`unsafe`**
- `unsafe_code = "forbid"`. If `unsafe` is ever really needed (FFI, a measured hot path) it needs the
  maintainer's approval first, then: its own module, a `// SAFETY:` comment on every block, and tests under Miri
  (`make miri`).

**Untrusted input never panics**
- All file contents (PBO, config, P3D, PAA, keys, signatures, include files) and CLI input are untrusted.
  Library code returns errors instead of panicking: no `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!` or
  unchecked slice indexing on such data (`unwrap_used`, `expect_used`, `panic`, `indexing_slicing` are lints;
  `clippy.toml` allows them in tests).
- Sizes and offsets read from files use checked arithmetic (`checked_add`, `checked_mul`) and `TryFrom`
  conversions. No truncating or sign-changing `as` casts (`cast_possible_truncation`, `cast_sign_loss`,
  `cast_possible_wrap` are denied). The release profile keeps `overflow-checks = true`.
- Never allocate from a length field before checking it against the remaining input and a sane limit
  (`Vec::with_capacity(len_from_file)` is a bug). Stream large files instead of reading them whole.
- Recursion on file-controlled depth (config classes, preprocessor includes and macros) has a depth limit.
- Floating point: no `==` on floats (`clippy::float_cmp`); sort with `total_cmp`.

**Files, paths and processes**
- PBO entry names are validated before use: reject absolute paths, drive letters, `..` components and empty
  names; normalise `\` separators; only then join to an output directory.
- Write build output atomically where a half-written file would be harmful (temp file in the same directory,
  then rename).
- Run external tools with `std::process::Command` and an argument list, never through a shell. Check the exit
  status, put stderr into the error.
- Check for missing external tools up front and name the tool and the setting to change.

**Errors and output**
- Library code returns `std::io::Error` built with the `error!` macro and adds context with `prepend_error`
  (file, offset, object). `main`/`run` decide whether to print and exit.
- Don't swallow errors: `let _ = fallible()` and `.ok()` on a `Result` need a comment saying why.
- Only the CLI layer prints (`print_stdout`/`print_stderr` are lints; the places that print carry
  `#[expect(..., reason)]`). The library never prints: a function that can warn takes a last parameter
  `warnings: &mut dyn WarningSink`; the CLI sink (`src/cli_warnings.rs`) prints, the MCP server collects.
- **Same input, same output:** builds are byte-for-byte reproducible. No `HashMap`/`HashSet` iteration order in
  output (sort first or use an ordered map), no timestamps in generated files unless a format requires them.

**Concurrency**
- No `static mut`, no new global mutable state; pass state explicitly. Warnings go to a `WarningSink` passed to
  the call (`dayzmake_core::warning`), never to a global.

**Secrets**
- Signing keys (`*.biprivatekey`) and other secrets never go into the repo, logs or error messages. The one
  throwaway test key under `tests/fixtures/bi/sign/` is the exception. `secrets/` is in `.gitignore`.

**Dependencies**
- **Look for a crate first (strict).** Before implementing a feature, parser, algorithm or helper, check whether
  a suitable crate exists (crates.io, lib.rs, RustSec). Writing it ourselves is only allowed when none exists or
  every candidate fails the review below; the commit message says which were considered and why rejected.
- **Review every crate before using it (strict).** Read its source, not just the README:
  - *security*: open RustSec advisories, handling of untrusted input (panics, unchecked indexing, allocation
    from length fields, unbounded recursion), justified `unsafe`, no surprising build scripts;
  - *maintenance*: recent releases, issues answered, not deprecated, archived or yanked, healthy bus factor;
  - *our standards*: tests, errors instead of panics, builds on Linux and Windows with Rust 1.99, deterministic
    output, a licence on the `deny.toml` allow-list (GPL-2.0-or-later compatible).
  A crate that fails the review is not used. The review result goes into the commit message that adds it.
- Add dependencies with `cargo add` (latest stable), only the features needed. No git or path dependencies, except workspace-internal crates (with a `version`).
- `cargo deny` checks licences, RustSec advisories, duplicates and sources; run `make deny` after any
  `Cargo.toml` change.

## Library rules (`dayzmake-core`)

`crates/dayzmake-core` is a public library other tools (for example Náves, AGPL-3.0) link.

- It never prints, never calls `process::exit`, and reads no environment variables except to locate and run external tools in
  `binarize` (`BIOUTPUT`, `STEAM_ROOT`, `HOME`, `DAYZ_TOOLS`, `PROTON`); new settings go into options structs. Warnings go to a
  `WarningSink` passed by the caller; there is no global warning state, and each caller thread uses its own sink.
- New parameters of a public function go into an options struct (like `PackOptions`), not into more arguments.
- Public data types and enums are `#[non_exhaustive]` and built through constructors or builders.
- No `HashMap`, `HashSet` or `LinkedHashMap` in the public API (iteration order, third-party types): return
  slices, iterators or ordered `Vec`s.
- `dayzmake-core` has its own semver, independent of the CLI. A removed or changed public item is a breaking
  change; the CLI depends on a compatible `dayzmake-core` version.
- Features `sign` and `image` stay additive: the crate builds, tests and documents without them.

## Documentation

- Public items have rustdoc comments (`missing_docs` is on); `make doc` builds them with warnings as errors.
- A commit that changes behaviour or the CLI updates `README.md` and the `USAGE` text in `src/run.rs` together, and
  the HTML documentation in `docs/book/src/reference/` and `docs/book/src/guides/` (a command, option, MCP tool or
  warning without its page fails `tests/book_sources.rs`; examples and described behaviour are not checked, so
  re-run them).

## Commits

- One logical change per commit, with `make check` passing.
- Conventional prefixes: `feat:`, `fix:`, `test:`, `docs:`, `build:`, `ci:`, `refactor:`, `chore:`, `release:`.
- The commit includes its tests and the documentation update it needs.

## Before pushing

Run `make check`. It must pass, because CI runs the same steps:

| Step | Target | Notes |
|---|---|---|
| formatting | `make fmt-check` | fix with `make fmt` |
| lint | `make lint` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| rustdoc | `make doc` | `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS="-D warnings"` |
| dependencies | `make deny` | `cargo deny check`: licences, advisories, bans, sources |
| features | `make features` | `dayzmake-core` builds with no features, `sign` only and `image` only |
| docs.rs | `make docsrs` | `cargo +nightly doc -p dayzmake-core --all-features` with `--cfg docsrs` and warnings as errors; skipped without nightly |
| book | `make book` | `mdbook build docs/book` into the target dir; skipped without `mdbook`. `tests/book_sources.rs` (in `make test`) fails when a command, option, MCP tool or warning has no page in `docs/book/src/reference/` |
| packaging | `make package-check` | `cargo package --workspace`; CI runs it without `--allow-dirty` |
| tests | `make test` | all targets plus doctests; includes the fuzz seeds |
| fuzzing | `make fuzz` | `cargo +nightly fuzz`, 10 s per target (`FUZZ_TIME=60` for longer) |
| coverage | `make coverage-check` | `cargo llvm-cov`; total coverage must stay at or above 70 % |

CI additionally runs **CodeQL** (`.github/workflows/codeql.yml`). New alerts block the merge; fix them or
dismiss them in GitHub with a written reason. When the change touches `unsafe` code, also run `make miri`.

SPDX/REUSE headers are not used in this repository (the files are mostly inherited from armake2 and keep their
original copyright holders); the licence is declared once, in `Cargo.toml` and `LICENSE`.

## Releasing

Publishing is done by the maintainer, or by an agent only when the maintainer explicitly asks for that release (and
only after CI on `main` is green).

1. Bump the versions. `dayzmake-core` follows semver on its own; a breaking core change also bumps the
   `version` in the CLI's `dayzmake-core` dependency.
2. `make check` (includes `make features` and `make package-check`).
3. Tag the release. The tag starts the release workflow, which builds the archives, signs their checksums with the
   release key (see below) and creates the GitHub release. If the signing step fails, no release is created.
4. `cargo publish -p dayzmake-core`, **then** `cargo publish -p dayzmake` (the CLI depends on the published core).

### Release signing key

The checksums of every release (`SHA256SUMS`, `SHA512SUMS`, with detached signatures `.asc`) are signed by
`scripts/sign-release.sh` in the `publish` job of `.github/workflows/release.yml`.

- Key: `dayzmake release signing key (https://github.com/bzed/dayzmake)`, ed25519, sign only, expires
  **2029-11-15**, fingerprint `6624 41B3 6A57 85AD 18FB  8667 DF75 3D99 D064 EB7E`.
- The public key is `docs/book/src/release-key.asc` (published at `https://bzed.github.io/dayzmake/release-key.asc`
  and attached to every release as `dayzmake-release-key.asc`).
- The private key is passphrase-protected and exists only in two repository secrets: `RELEASE_GPG_PRIVATE_KEY`
  (the ASCII-armoured secret key) and `RELEASE_GPG_PASSPHRASE`. It is never committed, logged or put on a command
  line; the script reads both from the environment and passes the passphrase on a file descriptor.
- After changing the script, run `scripts/test-sign-release.sh` (throwaway key, no CI job).
- Extending the expiry (do this well before 2029-11-15): on a trusted machine import the secret key, run
  `gpg --edit-key 662441B36A5785AD18FB8667DF753D99D064EB7E`, `expire`, choose the new date, `save`; export the public
  key with `gpg --armor --export 662441B36A5785AD18FB8667DF753D99D064EB7E > docs/book/src/release-key.asc` and
  commit it; export the secret key with `gpg --armor --export-secret-keys 662441B36A5785AD18FB8667DF753D99D064EB7E`
  and store it as the new value of the secret `RELEASE_GPG_PRIVATE_KEY` (the passphrase stays). Update the expiry date
  here and in the README section *Verifying downloads*, and delete the exported secret key afterwards. Users who
  imported the old public key re-import the new one; the fingerprint does not change.
- An expired key, an empty secret or a wrong passphrase make the signing step, and so the release, fail.
