# Environment and files

Everything dayzmake reads from its environment, and the files and folders it looks for, in one place. No
command reads any other variable. The library crate `dayzmake-core` reads variables only to locate and run
`binarize.exe`; everything else is an option of the caller.

## Environment variables

| Variable | Read by | Meaning |
|---|---|---|
| `HOME` | `api`, `build`, `binarize` | Home directory. Used to look for Steam (see below) and as the fallback for the API cache. |
| `STEAM_ROOT` | `api`, `build --proton-binarize`, `binarize` | The Steam directory, checked before every other location. It must contain `steamapps`. |
| `DAYZ_TOOLS` | `build --proton-binarize`, `binarize` | The DayZ Tools directory (the one that contains `Bin/Binarize/binarize.exe`). Default: `<steam>/steamapps/common/DayZ Tools`. |
| `PROTON` | `build --proton-binarize`, `binarize` | A Proton directory (the one that contains the `proton` script). Default: the first of `Proton Hotfix`, `Proton - Experimental`, `Proton Experimental` in `steamapps/common`, then the alphabetically last folder starting with `Proton` that has a `proton` script. |
| `BIOUTPUT` | `build`, `binarize` | `1` shows the output of `binarize.exe` (and Proton) while models are binarized. Without it the output is only shown when a run fails. |
| `DAYZMAKE_CACHE` | `api`, `mcp` | The folder of the script API cache. |
| `XDG_CACHE_HOME` | `api`, `mcp` | Linux and macOS: cache folder is `$XDG_CACHE_HOME/dayzmake/api` when `DAYZMAKE_CACHE` is not set. |
| `LOCALAPPDATA` | `api`, `mcp` | Windows: cache folder is `%LOCALAPPDATA%\dayzmake\api` when `DAYZMAKE_CACHE` is not set. |
| `TMPDIR` (`TEMP` on Windows) | `build`, `binarize` | Where the temporary folder of a binarize run goes. The Proton sandbox of a binarize run is a folder `dayzmake-binarize-<pid>-<n>` below it; it is removed when the run ends. |
| `ProgramFiles(x86)`, `ProgramFiles` | `api` (Windows) | Searched for `Steam\steamapps` after the registry. |
| `NO_COLOR` | all commands | Any value, even an empty one, switches the colours of warning and error messages off. |
| `CLICOLOR` | all commands | `0` switches colours off. |
| `CLICOLOR_FORCE` | all commands | A value other than `0` switches colours on even when the output is piped, and wins over `NO_COLOR`. |

`NO_COLOR`, `CLICOLOR` and `CLICOLOR_FORCE` are read by the `colored` library, not by dayzmake. Warnings and
errors are written to standard error, but whether they are coloured depends on **standard output** being a terminal
(unless `CLICOLOR_FORCE` is set): `dayzmake rapify x.cpp > /dev/null` in a terminal prints plain warnings, and
`dayzmake rapify x.cpp 2>err.txt` in a terminal writes escape codes into `err.txt`.

An empty `STEAM_ROOT`, `DAYZ_TOOLS`, `PROTON`, `DAYZMAKE_CACHE`, `XDG_CACHE_HOME`, `LOCALAPPDATA`, `ProgramFiles(x86)`
or `ProgramFiles` counts as unset. `HOME` is not checked: an empty `HOME` makes the Steam and cache locations
relative paths (`.steam/steam`, `.cache/dayzmake/api`), so set it properly. `BIOUTPUT` only counts when it is
exactly `1`.

## How the Steam installation is found

`api` collects candidate Steam folders in this order and keeps those that have a `steamapps` folder, without
duplicates: `$STEAM_ROOT`, then below `$HOME`: `.steam/steam`, `.steam/root`, `.steam/debian-installation`,
`.local/share/Steam`, the Flatpak folder `.var/app/com.valvesoftware.Steam/.local/share/Steam`, the Snap folder
`snap/steam/common/.local/share/Steam` and macOS's `Library/Application Support/Steam`. On Windows the registry
value `HKEY_CURRENT_USER\Software\Valve\Steam`, `SteamPath` comes next, then `%ProgramFiles(x86)%\Steam` and
`%ProgramFiles%\Steam`.

Each candidate contributes its library folders too: `steamapps/libraryfolders.vdf` is read and every `path` in it
that is an existing absolute directory is searched for app manifests (`appmanifest_<id>.acf`). The first library
with a manifest whose install folder exists wins. These files are untrusted: they are read at most 1 MiB, with
invalid UTF-8 replaced, and an `installdir` must be a safe relative path.

| Branch | Client app | Server app |
|---|---|---|
| stable | 221100 | 223350 |
| experimental | 1024020 | 1042420 |

The game's scripts are `dta/scripts.pbo` of the client install, or else of the server install, whichever has
the file; `--dayz <dir>` replaces the search. Workshop mods are
`steamapps/workshop/content/<client app>/<workshop id>/`. A mod folder has `addons/*.pbo` (the `addons` folder is
found ignoring case), `keys`, and optionally a `meta.cpp` whose `name = "...";` is the mod's name. The
`@name` folders in the game's install (where the DayZ launcher links mods) are listed too.

`binarize` and `build --proton-binarize` search a shorter list for the Steam folder: `$STEAM_ROOT`, then
`~/.steam/debian-installation`, `~/.steam/steam` and `~/.local/share/Steam`; the first that has
`steamapps/common` is used. DayZ Tools is Steam app 830640.

## Windows registry

| Key | Value | Used for |
|---|---|---|
| `HKEY_CURRENT_USER\Software\Valve\Steam` | `SteamPath` | Finding Steam for the script API. |
| `HKEY_CURRENT_USER\Software\Bohemia Interactive\binarize` | `path` | The folder of `binarize_x64.exe`, which is what `build` and `binarize` run on Windows. |

## Files and folders dayzmake reads or writes

| Path | What |
|---|---|
| `$PBOPREFIX$`, `$PREFIX$` | In a source folder: the prefix and header extensions of the PBO (see [Header extensions](header-extensions.md)). `unpack` writes `$PBOPREFIX$`. |
| `$NOBIN$`, `$NOBIN-NOTEST$` | In a source folder: switch rapification and binarization off for `build`. |
| `<cache>/vanilla-<branch>.json` | The index of the game scripts. `<cache>` is `$DAYZMAKE_CACHE`, else `$XDG_CACHE_HOME/dayzmake/api`, else `$HOME/.cache/dayzmake/api` (`%LOCALAPPDATA%\dayzmake\api` on Windows). The folder is created with mode `0700`. |
| `<cache>/mod-<name>-<branch>-<hash>.json` | The index of one mod. |
| `*.biprivatekey`, `*.bikey`, `*.bisign` | Keys and signatures. Private keys are written with mode `0600`. |

A cache file is rebuilt when the size or modification time of one of its PBOs changed or the format of the index
changed, and a corrupt file is replaced. A cache folder that cannot be found or written is a warning
([`api-cache`](warnings.md#api-cache)); the query still runs on the index that was built in memory. Cache files can
be deleted at any time; `--refresh` and `api update` rebuild them.

## Network

dayzmake never uses the network.
