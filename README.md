dayzmake
========

[![](https://github.com/bzed/dayzmake/actions/workflows/ci.yml/badge.svg)](https://github.com/bzed/dayzmake/actions/workflows/ci.yml)
[![](https://github.com/bzed/dayzmake/actions/workflows/codeql.yml/badge.svg)](https://github.com/bzed/dayzmake/actions/workflows/codeql.yml)
[![](https://img.shields.io/badge/license-GPL--2.0--or--later-blue.svg)](LICENSE)

DayZ modding tools written in Rust: PBO packing and unpacking, config
preprocessing and rapification, model inspection and binarization, PAA
conversion and signature handling.

## History

dayzmake is a fork of [armake2](https://github.com/KoffeinFlummi/armake2)
itself the successor to armake, diverged from it for DayZ modding: Arma 3
targets are no longer the focus, while DayZ-specific behaviour (binarization
through DayZ Tools under Proton, `product=dayz` header extensions and similar)
is maintained here.

**Status:** PAA commands not implemented, some options not implemented, testing.

## Building

The build requires `cargo`, Rust's package manager and the OpenSSL development libraries.
To compile and run, use:

```
cargo run
```

To build a release, use:

```
cargo build --release
```

In order to build, you'll need to have OpenSSL installed on your system.

On **Linux**, the easiest way is to install OpenSSL via your system's package manager (if it is not installed already). Make sure you also have the development packages of OpenSSL installed. For example, `libssl-dev` on Ubuntu or `openssl-devel` on Fedora.

On **Windows**, the easiest way to get compilation and static linking of OpenSSL to work is to download [pre-compiled OpenSSL binaries](http://slproweb.com/products/Win32OpenSSL.html) (non-light, 64-bit) and set the following environment variables:

- `OPENSSL_DIR=C:\OpenSSL-WIN64`
- `OPENSSL_STATIC=1`
- `OPENSSL_LIBS=libssl_static:libcrypto_static`

## Testing

Unit tests:

```
cargo test
```

The end-to-end DayZ test harness builds a small test mod, binarizes it, packs
it into a PBO, signs it, and boots a local DayZ server (Steam app 223350, plus
app 1042420 with `--both`) to prove the mod loads and runs:

```
testharness/run.sh --both
```

See [testharness/README.md](testharness/README.md) for details.

## Usage

```
dayzmake

Usage:
    dayzmake rapify [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
    dayzmake preprocess [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
    dayzmake derapify [-v] [-f] [-d <indentation>] [<source> [<target>]]
    dayzmake binarize [-v] [-f] [-w <wname>]... <source> <target>
    dayzmake build [-v] [-f] [--proton-binarize] [-w <wname>]... [-i <includefolder>]... [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
    dayzmake pack [-v] [-f] <sourcefolder> [<target>]
    dayzmake inspect [-v] [<source>]
    dayzmake unpack [-v] [-f] <source> <targetfolder>
    dayzmake cat [-v] <source> <filename> [<target>]
    dayzmake keygen [-v] [-f] <keyname>
    dayzmake sign [-v] [-f] [--v2] <privatekey> <pbo> [<signature>]
    dayzmake verify [-v] <publickey> <pbo> [<signature>]
    dayzmake paa2img [-v] [-f] <source> <target>
    dayzmake img2paa [-v] [-f] [-c] [-t <paatype>] <source> <target>
    dayzmake (-h | --help)
    dayzmake --version
```

See `dayzmake --help` for more.

## Inspecting models and checking their textures

`dayzmake inspect <file.p3d>` shows the format and version of a model. For an editable MLOD it also lists the LODs, textures, materials, properties and selections; for a binarized ODOL (or BMTR) only the format and version are known. Without a model header, `inspect` treats the input as a PBO.

`build` checks every MLOD model it is about to binarize (also without `--proton-binarize`) and warns about textures and materials that are not in the addon (`missing-texture`, `missing-material`; mute with `-w`). References starting with the addon's prefix are looked up in the source folder, ignoring case. Other references (vanilla `dz\...`, other mods) are only checked if they exist below an `-i` folder or the current directory; if they don't, they are not reported because there is no way to tell whether they are wrong. Procedural textures (`#(argb,...)`) are skipped.

## Binarizing models on Linux

BI's `binarize.exe` only runs on Windows, so on Linux `build` copies `.p3d`/`.rtm` files as-is (with the `non-windows-binarization` warning). With `--proton-binarize` (and for `dayzmake binarize`) dayzmake runs the DayZ Tools `binarize.exe` under Proton instead. Config and rvmat rapification is unchanged. Files that are already binarized (`ODOL` models, `BMTR` animations) never reach `binarize.exe`; they are copied as-is with the `prebinarized-file` warning, which names the format and version (e.g. `ODOL v56`). Any version is accepted; dayzmake does not know which ones the game loads.

Requirements: DayZ Tools (Steam app 830640) and any Proton installed through Steam.

- `STEAM_ROOT`: Steam directory. Default: first of `~/.steam/debian-installation`, `~/.steam/steam`, `~/.local/share/Steam` that has `steamapps/common`.
- `DAYZ_TOOLS`: DayZ Tools directory. Default: `<steam>/steamapps/common/DayZ Tools` (uses `Bin/Binarize/binarize.exe`).
- `PROTON`: Proton directory (containing the `proton` script). Default: `Proton Hotfix`, `Proton - Experimental`, `Proton Experimental`, then any `Proton*` in `steamapps/common`.
- `BIOUTPUT=1`: show the output of Proton/binarize.exe.

dayzmake builds a throw-away sandbox in the temp directory (a copy of `Binarize`, a fresh Wine prefix and an output folder) once per run and removes it at the end. Wine's default `Z:` drive maps to `/`, and `binarize.exe` walks directory trees from the drive root, so it would run for minutes on a real home directory; the sandbox therefore points `Z:` at itself. The first model of a run is slow because the Wine prefix has to be created (up to a few minutes on a cold start). Nothing in the Steam installation is modified.

`binarize.exe` resolves texture and material paths (`my\mod\data\x_ca.paa`) relative to the project root, and only sets a model's alpha flags if it can read the texture. dayzmake therefore gives the sandbox a `P:` drive with the addon linked at its PBO prefix (`P:\my\mod` → the source folder) and runs `binarize.exe` from `P:\`. Paths outside the addon (vanilla `dz\...`) stay unresolved and are reported as `binarize-missing-texture`. The source folder is linked, not copied, so `binarize.exe` can read and in principle write it; its output goes to the sandbox, and the source is not modified.

`dayzmake binarize <source> <target>` has no PBO to take the prefix from: it treats the model's folder as the addon and uses the folder name as the prefix. That only works for a model in the addon root of a single-component prefix; for anything else (a model in `data\`, or a prefix like `my\mod`) use `build --proton-binarize`, which takes the prefix from `$PBOPREFIX$`.
