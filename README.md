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
    dayzmake pack [-v] [-f] [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
    dayzmake inspect [-v] [<source>]
    dayzmake unpack [-v] [-f] <source> <targetfolder>
    dayzmake cat [-v] <source> <filename> [<target>]
    dayzmake keygen [-v] [-f] <keyname>
    dayzmake sign [-v] [-f] [--v2] <privatekey> <pbo> [<signature>]
    dayzmake paa2img [-v] [-f] <source> <target>
    dayzmake img2paa [-v] [-f] [-c] [-t <paatype>] <source> <target>
    dayzmake verify [-v] <publickey> <pbo> [<signature>]
    dayzmake api class|find|sub|enum|chain|src|defines|update|mods ...  (see "Script API index")
    dayzmake mcp [-v]
    dayzmake (-h | --help)
    dayzmake --version

Commands:
    rapify      Preprocess and rapify a config file.
    preprocess  Preprocess a file.
    derapify    Derapify a config.
    binarize    Binarize a file using BI's binarize.exe (Windows; on Linux through Proton).
                The model's folder is the addon and its name the prefix; use build
                for models whose textures are referenced by a longer prefix.
    build       Build a PBO from a folder.
    pack        Pack a folder into a PBO without any binarization or rapification.
    inspect     Inspect a PBO and list contained files, or a model (MLOD/ODOL) and
                show its format, version, LODs, textures, materials and selections.
    unpack      Unpack a PBO into a folder.
    cat         Read the named file from the target PBO to stdout.
    keygen      Generate a keypair with the specified path (extensions are added).
    sign        Sign a PBO with the given private key.
    verify      Verify a PBO's signature with the given public key.
    paa2img     Convert a PAA file to a standard image format (e.g. PNG).
    img2paa     Convert a standard image file to PAA.

Options:
    -v --verbose                Enable verbose output.
    -f --force                  Overwrite the target file/folder if it already exists.
    -w --warning <wname>        Warning to disable
    -i --include <includefolder>    Folder to search for includes, defaults to CWD.
    -x --exclude <excludepattern>   Glob pattern to exclude from PBO.
                                      For unpack: pattern to exclude from output folder.
    -d --indent <indentation>   String to use for indentation. 4 spaces by default.
    -e --headerext <headerext>  Extension to add to PBO header as "key=value".
    -k --key <privatekey>       Sign the PBO with the given private key.
    -c --compress               Compress the output PAA file.
    -t --type <paatype>         PAA compression type (e.g. dxt1, dxt5). [default: dxt5]
    -s --signature <signature>  Signature path to use when signing the PBO.
    --v2                     Generate an older v2 signature.
    --proton-binarize           Linux: binarize .p3d/.rtm with DayZ Tools' binarize.exe under Proton.
    -h --help                   Show usage information and exit.
       --version                Print the version number and exit.
```

See `dayzmake --help` for more.

## Script API index

`dayzmake api` answers questions about the DayZ script API (classes, members, enums, base class chains, where a
declaration lives, compile-time defines) from the installed game, without extracting anything.

```
dayzmake api class PlayerBase --max 40        # members of a class
dayzmake api class PlayerBase --inherited     # with its base classes
dayzmake api find GetGroup -m cf              # regex over members, classes and enum items
dayzmake api sub ManBase --all                # subclasses
dayzmake api enum EStatLevels
dayzmake api chain PlayerBase
dayzmake api src AIAgent                      # file and line of every declaration
dayzmake api src AIAgent --cat                # the full source files
dayzmake api defines [regex]
dayzmake api update                           # rebuild the cache now
```

- **Branch:** stable by default, `--experimental` for the experimental branch. The two branches have separate
  indexes and caches. The install is found through Steam (`STEAM_ROOT` first; on Windows then the registry's `SteamPath`, `%ProgramFiles(x86)%\Steam` and `%ProgramFiles%\Steam`); `--dayz <dir>` names a DayZ client or
  server directory (`<dir>/dta/scripts.pbo`) and skips the Steam search.
- **Mods:** without `-m` only vanilla is queried. `-m cf,cot` (aliases `cf`, `cot`, `dabs`, `expansion`), a Steam
  workshop id, a mod name (`meta.cpp` name or `@dir`, see `dayzmake api mods`) or a mod directory adds mods;
  `-m aliases` means the four aliases that are installed, `-m all` every installed mod that has scripts (the first
  run indexes each mod once, which can take a while; later runs use the cache). A mod contributes the
  scripts of every `addons/*.pbo` (the `.c` files, with the `defines[]` of its `config.cpp`/`config.bin`).
  Results from several indexes are tagged with the index name.
- **Output:** text, at most `--max` lines (default 120). `--json` prints JSON instead (errors stay on stderr as
  text); `-v` first prints one `# <index>: <build>` line per index.
- **Cache:** one JSON file per index in `$DAYZMAKE_CACHE`, else `$XDG_CACHE_HOME/dayzmake/api`, else
  `~/.cache/dayzmake/api` (Windows: `%LOCALAPPDATA%\dayzmake\api`). It is rebuilt automatically when the size or
  modification time of a PBO changes or the index format changes; `--refresh` forces it. A corrupt file is replaced. A cache that cannot be found or written (read-only directory, no `HOME`) is a warning
  (`api-cache`); the query still runs on the freshly built index.
- PBO contents are untrusted: entries and prefixes with unsafe paths are skipped with a warning and nothing is
  extracted to disk.

## MCP server

`dayzmake mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server on stdin/stdout
(newline-delimited JSON-RPC, tools only). Register it with Claude Code:

```
claude mcp add dayzmake -- dayzmake mcp
```

Tools:

- `api_class`, `api_find`, `api_subclasses`, `api_enum`, `api_chain`, `api_source`, `api_defines`, `api_update`:
  the script API index (see above). They take `mods`, `experimental`, `dayz`, `refresh`, `json` and `max`
  (default 120). Without `mods` only vanilla is queried; no tool defaults to `all`.
- `pbo_inspect`, `pbo_cat`, `pbo_unpack`, `pbo_pack`, `pbo_build`, `pbo_sign`, `pbo_verify`, `key_generate`,
  `config_rapify`, `config_derapify`, `config_preprocess`, `paa_to_image`, `image_to_paa`, `binarize`: the file
  commands with the arguments of their CLI counterparts. No tool overwrites a file unless `force` is set.

The tools take file paths as given (relative to the server's working directory) and act with the permissions of
the user running the server: they can read, create and, with `force`, overwrite any file that user can. Only
register the server with a client you trust to do that. Output is capped at 200 000 bytes; `api_source` (with `cat`) and `pbo_cat` take `offset` and `length`
(bytes) to fetch the rest, and `api_source` takes `target` to write the files instead. Warnings come first in
the result, so a cut never drops them, and failures are results with `isError: true`.

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
