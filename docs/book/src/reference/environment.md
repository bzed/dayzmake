# Environment and files

This page lists everything that dayzmake reads from its environment, and the files and folders that it looks for.
No command reads any other variable. The library crate `dayzmake-core` reads variables only to find and run
`binarize.exe`. For everything else, the caller gives an option.

## Environment variables

| Variable | Read by | Meaning |
|---|---|---|
| `HOME` | `api`, `build`, `binarize` | The home directory. dayzmake uses it to look for Steam (see below) and as the fallback for the API cache. |
| `STEAM_ROOT` | `api`, `build --proton-binarize`, `binarize` | The Steam directory. dayzmake checks it before every other location. It must contain `steamapps`. |
| `DAYZ_TOOLS` | `build --proton-binarize`, `binarize` | The DayZ Tools directory (the one that contains `Bin/Binarize/binarize.exe`). Default: `<steam>/steamapps/common/DayZ Tools`. |
| `PROTON` | `build --proton-binarize`, `binarize` | A Proton directory (the one that contains the `proton` script). Default: the first of `Proton Hotfix`, `Proton - Experimental`, `Proton Experimental` in `steamapps/common`, then the alphabetically last folder starting with `Proton` that has a `proton` script. |
| `BIOUTPUT` | `build`, `binarize` | `1` shows the output of `binarize.exe` (and Proton) while dayzmake binarizes models. Without it, dayzmake shows the output only when a run fails. |
| `DAYZMAKE_CACHE` | `api`, `mcp` | The folder of the script API cache. |
| `XDG_CACHE_HOME` | `api`, `mcp` | Linux and macOS: the cache folder is `$XDG_CACHE_HOME/dayzmake/api` when `DAYZMAKE_CACHE` is not set. |
| `LOCALAPPDATA` | `api`, `mcp` | Windows: the cache folder is `%LOCALAPPDATA%\dayzmake\api` when `DAYZMAKE_CACHE` is not set. |
| `TMPDIR` (`TEMP` on Windows) | `build`, `binarize` | The place for the temporary folder of a binarize run. The Proton sandbox of a binarize run is a folder `dayzmake-binarize-<pid>-<n>` below it. dayzmake removes it when the run ends. |
| `ProgramFiles(x86)`, `ProgramFiles` | `api` (Windows) | dayzmake searches them for `Steam\steamapps` after the registry. |
| `NO_COLOR` | all commands | Any value, even an empty one, switches off the colours of warning and error messages. |
| `CLICOLOR` | all commands | `0` switches colours off. |
| `CLICOLOR_FORCE` | all commands | A value other than `0` switches colours on even when the output is piped, and wins over `NO_COLOR`. |

The `colored` library reads `NO_COLOR`, `CLICOLOR` and `CLICOLOR_FORCE`, not dayzmake. dayzmake writes warnings and
errors to standard error. But the colours depend on **standard output**: it must be a terminal (unless
`CLICOLOR_FORCE` is set). `dayzmake rapify x.cpp > /dev/null` in a terminal prints plain warnings.
`dayzmake rapify x.cpp 2>err.txt` in a terminal writes escape codes into `err.txt`.

An empty `STEAM_ROOT`, `DAYZ_TOOLS`, `PROTON`, `DAYZMAKE_CACHE`, `XDG_CACHE_HOME`, `LOCALAPPDATA`, `ProgramFiles(x86)`
or `ProgramFiles` counts as unset. dayzmake does not check `HOME`. An empty `HOME` makes the Steam and cache
locations relative paths (`.steam/steam`, `.cache/dayzmake/api`), so set `HOME` correctly. `BIOUTPUT` counts only
when it is exactly `1`.

## How the Steam installation is found

`api` collects candidate Steam folders in this order. It keeps those that have a `steamapps` folder, without
duplicates: `$STEAM_ROOT`, then these folders below `$HOME`: `.steam/steam`, `.steam/root`,
`.steam/debian-installation`, `.local/share/Steam`, the Flatpak folder `.var/app/com.valvesoftware.Steam/.local/share/Steam`,
the Snap folder `snap/steam/common/.local/share/Steam` and, for macOS, `Library/Application Support/Steam`. On
Windows, the registry value `HKEY_CURRENT_USER\Software\Valve\Steam`, `SteamPath` comes next. Then come
`%ProgramFiles(x86)%\Steam` and `%ProgramFiles%\Steam`.

Each candidate also gives its library folders. dayzmake reads `steamapps/libraryfolders.vdf`. It searches every
`path` in that file that is an existing absolute directory for app manifests (`appmanifest_<id>.acf`). The first
library with a manifest whose install folder exists wins. These files are untrusted. dayzmake reads at most 1 MiB,
replaces invalid UTF-8, and accepts an `installdir` only if it is a safe relative path.

| Branch | Client app | Server app |
|---|---|---|
| stable | 221100 | 223350 |
| experimental | 1024020 | 1042420 |

The scripts of the game are `dta/scripts.pbo` of the client install. If the client install does not have the file,
dayzmake uses the server install. `--dayz <dir>` replaces the search. Workshop mods are in
`steamapps/workshop/content/<client app>/<workshop id>/`. A mod folder has `addons/*.pbo` (dayzmake finds the
`addons` folder and ignores case), `keys`, and optionally a `meta.cpp`. The `name = "...";` in `meta.cpp` is the
name of the mod. dayzmake also lists the `@name` folders in the install of the game (the DayZ launcher links mods
there).

`binarize` and `build --proton-binarize` search a shorter list for the Steam folder: `$STEAM_ROOT`, then
`~/.steam/debian-installation`, `~/.steam/steam` and `~/.local/share/Steam`. They use the first folder that has
`steamapps/common`. DayZ Tools is Steam app 830640.

## Windows registry

| Key | Value | Used for |
|---|---|---|
| `HKEY_CURRENT_USER\Software\Valve\Steam` | `SteamPath` | To find Steam for the script API. |
| `HKEY_CURRENT_USER\Software\Bohemia Interactive\binarize` | `path` | The folder of `binarize_x64.exe`. On Windows, `build` and `binarize` run this file. |

## Files and folders dayzmake reads or writes

| Path | What |
|---|---|
| `$PBOPREFIX$`, `$PREFIX$` | In a source folder: the prefix and header extensions of the PBO (see [Header extensions](header-extensions.md)). `unpack` writes `$PBOPREFIX$`. |
| `$NOBIN$`, `$NOBIN-NOTEST$` | In a source folder: they switch off rapification and binarization for `build`. |
| `<cache>/vanilla-<branch>.json` | The index of the game scripts. `<cache>` is `$DAYZMAKE_CACHE`, else `$XDG_CACHE_HOME/dayzmake/api`, else `$HOME/.cache/dayzmake/api` (`%LOCALAPPDATA%\dayzmake\api` on Windows). dayzmake creates the folder with mode `0700`. |
| `<cache>/mod-<name>-<branch>-<hash>.json` | The index of one mod. |
| `*.biprivatekey`, `*.bikey`, `*.bisign` | Keys and signatures. dayzmake writes private keys with mode `0600`. |

dayzmake rebuilds a cache file when the size or the modification time of one of its PBOs changed, or when the
format of the index changed. It replaces a corrupt file. If dayzmake cannot find or write the cache folder, it gives
a warning ([`api-cache`](warnings.md#api-cache)). The query still runs on the index that dayzmake built in memory.
You can delete cache files at any time. `--refresh` and `api update` rebuild them.

## Network

dayzmake never uses the network.
