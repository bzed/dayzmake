# Golden CLI fixtures

`mod/` is a small synthetic addon (written by hand for dayzmake, no game data). `tests/golden_cli.rs` copies it
into a temp dir, adds `tests/fixtures/mlod/small.p3d` as `data/model.p3d`, and records the output of every CLI
command in `tests/snapshots/`. The suite pins today's behaviour; a snapshot changes only on purpose.
The snapshot suites are unix-only (`#![cfg(unix)]`); Windows CLI output is not pinned.

- `$PBOPREFIX$`: the prefix only (no header extensions).
- `config.cpp`, `inc/macros.hpp`: macros, `__EVAL`, inheritance, `delete`, `#ifdef`, an unquoted string (warning).
- `data/mat.rvmat`: material referencing a texture that does not exist (missing-texture warning on build).
- `data/readme.txt`, `data/sub/deep.txt`: plain files.
