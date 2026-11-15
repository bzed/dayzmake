# dayzmake workspace split: `dayzmake-core` library + `dayzmake` CLI

## Why

Náves (AGPL-3.0, Rust workspace, `crates/naves-export`) runs `dayzmake` as a separate program for four things:
`keygen`, `img2paa` (DXT1/DXT5, many in parallel), `build -k` (pack and sign) and `unpack` (read road models from
`structures_roads.pbo`). Its users must install a second tool. Náves wants to link the code as a library instead.
Other projects should be able to do the same, so the library is designed for every format dayzmake handles, not
only the four Náves uses.

Licence: dayzmake is GPL-2.0-or-later; GPLv3 §13 allows combining with AGPLv3, so Náves may link it.

## Goals

- A library crate `dayzmake-core` with the format code: PBO, config (preprocess, rapify, derapify, `__EVAL`), P3D,
  PAA, BI keys and signatures, the binarize bridge. It never prints, has no global mutable state and no CLI
  dependencies (`docopt`, `colored`, `serde`, `serde_json`).
- The `dayzmake` crate (CLI, MCP server, script API index) on top of it, with **unchanged CLI and MCP behaviour**
  apart from the two bug fixes below.
- A public API a consumer can rely on: options structs instead of CLI-shaped arguments, no `HashMap`/`LinkedHashMap`
  in public fields, warnings collected per call, binarize as an explicit object.
- Separate versions: `dayzmake-core` starts at `0.1.0`; `dayzmake` keeps its own version.
- Proof of no breakage: a characterization suite pinning today's CLI and MCP output, recorded before the refactor, and
  a differential run against the v2.0.0 release on real game files at the end.

## Non-goals

- Typed error enums. Errors stay `std::io::Error` built with `error!` (AGENTS.md); a typed error is a later, breaking
  `0.x` release.
- Splitting the script API index (`api`) into its own crate. It moves with the CLI and only uses the public core API,
  so it can be split later.
- Publishing to crates.io. The plan makes `dayzmake-core` packageable (`cargo package`); publishing is the
  maintainer's call.
- Upgrading `image` (0.24) or other dependencies.
- Streaming PBO entries (the PBO stays in memory, as today).

## Anti-goals

- Any change to CLI output, exit codes, file bytes or MCP replies that is not one of the listed fixes.
- Weakening tests to get the refactor through: during the refactor existing tests may change imports, paths and call
  signatures, never their expected values (except in the two fix commits, which name what they change).

## Bugs found during the survey (fixed in this work)

1. `PBO::write` writes header extensions in `HashMap` order: a PBO with more than one extension besides `prefix` is
   not reproducible, and `unpack` writes `$PBOPREFIX$` lines in random order. Fix: keep file order.
2. The warning quota never lets the counter exceed the quota, so `print_warning_summary` never reports suppressed
   warnings. Fix: count every raised warning; the summary prints the excess.
3. (API trap, not a CLI bug) `BIPrivateKey::sign` fails on a PBO that was not written first ("PBO has no checksum").
   Fix: compute the checksum when it is missing.

## Approaches considered

- **A. One crate with Cargo features.** The least churn, but one version for CLI and library, so a library break
  forces a major tool release.
- **B. Workspace with two crates (chosen).** Separate versions, a compiler-enforced boundary, a small dependency tree
  for consumers. One-time cost to move files, tests, fuzz, benches, CI.
- **C. Use the crate as it is.** Consumers inherit the global state and an unpromised API. Rejected.

## Decided

- Layout: the repo root stays the `dayzmake` package and becomes the workspace root; the library lives in
  `crates/dayzmake-core`. Shared test fixtures stay in `tests/fixtures/`, fuzz corpora in `fuzz/corpus/`.
- Tests that run the `dayzmake` binary stay in the root `tests/`; pure library tests move to
  `crates/dayzmake-core/tests/`.
- Features of `dayzmake-core`: `sign` (OpenSSL) and `image` (image conversion for PAA), both on by default.
- Warnings: a `WarningSink` trait passed as `&mut dyn WarningSink`; `Warnings` collects with mute and per-name quota.
  The CLI prints immediately through its own sink; the MCP server uses one `Warnings` per call.
- Binarize: a `Binarizer` value owns the Proton sandbox and removes it on drop; `None` means "do not binarize".

## Left to the builder

Exact accessor bodies, the internal storage types behind the accessors, module splits inside the CLI crate beyond
those named in the plan, and the wording of new rustdoc.
