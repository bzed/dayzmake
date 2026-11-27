# dayzmake DayZ test harness

This is an end-to-end test for dayzmake against the current DayZ server (1.29 stable and
1.30 experimental). The test builds a small test mod, binarizes its config, packs it
into a PBO and signs it. Then it boots a real DayZ server with the mod loaded. This proves
that the PBO loads, the scripts compile and the code runs.

## What it does

1. `cargo build --release` (or uses `$DAYZMAKE_BIN`)
2. `dayzmake keygen` - generates `.biprivatekey` / `.bikey`
3. `dayzmake build` - preprocesses, rapifies `config.cpp` to `config.bin`
   (binarization), packs the addon into a PBO with `prefix=` header from the
   `$PREFIX$` file
4. `dayzmake sign` - V3 signature (`.bisign`)
5. `dayzmake verify` - checks the signature against the public key
6. `dayzmake unpack` round-trip check of the packed files
7. Boots the local DayZ server (Steam app 223350, and app 1042420 with
   `--experimental`/`--both`) twice from a disposable symlink tree:
   vanilla baseline, then with `-servermod=@dayzmake_test`
8. Fails unless the Mission script module grows by exactly one file, the
   test mod's `Print` marker appears in `script_*.log`, there are no script
   compile errors for the mod, and no signature errors in the RPT

## Usage

```sh
testharness/run.sh               # build + PBO checks + stable server test
testharness/run.sh --both        # also boot the experimental 1.30 server
testharness/run.sh --no-server   # build + PBO/signature checks only
testharness/run.sh --keep        # keep testharness/build for inspection
```

Requirements: Rust toolchain, a Steam installation with DayZ Server
(app 223350) for the server test, `python3` (port picking, `compare-release.sh`).

The harness never writes into the Steam install. It builds its own server
tree of symlinks in `testharness/build/server-tree-*`. It picks random free
game/query/RCon ports. It removes its own files when the run ends.

## Test mod

`mod/addons/dayzmake_test/` is a minimal script-only DayZ mod. It has a
`config.cpp` (`CfgPatches` + `CfgMods` with a `missionScriptModule`) and a
`$PREFIX$` file. It also has one Enforce script. This script overrides
`MissionServer.OnMissionStart()` to print a marker. The mod uses only APIs
present in both 1.29 and 1.30. So the same PBO loads on both versions.

## Notes

- On 1.30 experimental the server ends with a vanilla shutdown assertion
  (`Leaked 'BunkerBroadcastManager'`). This is vanilla noise. The test mod does
  not cause it, and it does not fail the harness.
- The engine logs `Can't load @dayzmake_test/Anims/cfg/skeletons.anim.xml`
  style lines while it probes each loaded mod folder for animation configs.
  These lines are expected for a script-only mod.

## Differential comparison against a released dayzmake

`compare-release.sh <old-dayzmake> <new-dayzmake> <dayz-dir>` runs the same commands with two binaries and
compares stdout, stderr, exit codes and every file written. It is manual (not run by CI) and is the final
check that a refactor changed no CLI behaviour. `<dayz-dir>` (a DayZ server or client install) is only read.

```sh
git worktree add /var/tmp/dayzmake-v2/src v2.0.0
CARGO_TARGET_DIR=/var/tmp/dayzmake-v2/target cargo build --release --locked --manifest-path /var/tmp/dayzmake-v2/src/Cargo.toml
CARGO_TARGET_DIR=/var/tmp/dayzmake-target cargo build --release --locked
testharness/compare-release.sh /var/tmp/dayzmake-v2/target/release/dayzmake \
    /var/tmp/dayzmake-target/release/dayzmake ~/.steam/steam/steamapps/common/DayZServer
git worktree remove --force /var/tmp/dayzmake-v2/src   # also after a failed run
rm -rf /var/tmp/dayzmake-v2
```

It compares, per `addons/*.pbo`: `inspect` and `unpack`; `derapify` of every unpacked `config.bin`; `inspect`
of the first 50 `.p3d`; and every command of `tests/golden_cli.rs` on `tests/fixtures/golden/mod`, plus
`img2paa` of 1024x1024 gradient and noise PNGs. The work folder (`mktemp -d /var/tmp/dayzmake-compare.XXXXXX`)
is removed unless `KEEP=1`. The summary lists `N identical, M different` for each category. A difference that is
only the header extension block of `inspect`/`$PBOPREFIX$` in another order, or build warnings in another
order (golden commands only), is marked `order-only`. Every other byte must match. So a reordered entry list or
config member is a plain difference. The `raw` category checks, for each PBO, that the new binary lists header
extensions in the order of the PBO file. A released dayzmake that predates the order fix writes them in a random
order for each run. So the number of identical `pbo` comparisons varies between runs. Exit status 1 means that
something differs.

Against v2.0.0, these differences are expected. Header extension order changes (file order now, random before).
Warning order changes (sorted file order now). Suppression summary lines appear on stderr. PBO bytes differ when
more than one `-e` header extension is given.
