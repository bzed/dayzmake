# Warnings

Every warning has a name, shown in square brackets at the end of its line. Mute a warning with `-w <name>` on
the commands that accept it (`rapify`, `preprocess`, `binarize`, `build`; the warnings of `pack`, `unpack` and `api` cannot be muted); in the MCP server warnings are returned
with the result of each tool call. At most ten warnings of the same name are shown per run; use `-v` to see them
all. A warning never changes the exit status: only errors do.

The messages below are real output (shortened paths are marked with `…`), except where a note says that the
format is quoted from the source because the situation needs files this manual cannot ship (DayZ Tools, a damaged
PBO of the game).

Each section lists the commands that can raise the warning, what triggers it and an example.

## api-bad-config

Raised by: `api`, `mcp`.

A `config.bin` or `config.cpp` cannot be read. An `#include` in a `config.cpp` is only resolved from the entries of the same PBO (relative to the including entry, or as `\prefix\path` inside the prefix of the PBO): an absolute path, a drive letter, a `..` that would leave the PBO's root or a file that is not in the PBO makes the config unreadable (`..` inside the PBO is resolved against the folder of the including entry), and the file system is never read. For a mod PBO its `defines[]` are not collected (the scripts of the PBO are still indexed); for `api config` the config is left out of the config index. A class with a name that cannot be a path segment (empty, with a `/` or a control character) is skipped with the same warning.

```text
In file …/badmod/addons/entry.pbo: warning: cannot read config.bin: failed to fill whole buffer [api-bad-config]
```

## api-bad-entry

Raised by: `api`, `mcp`.

An entry of a PBO that holds scripts cannot be read (for example a corrupt compressed entry). The entry is skipped.

```text
warning: cannot read "<entry name>": <reason> [api-bad-entry]   (the message format; no synthetic file in this manual triggers it)
```

## api-bad-mod

Raised by: `api`, `mcp`.

A selected mod cannot be indexed, for example because its folder disappeared, or an indexing thread failed. The mod is skipped and the others are answered.

```text
warning: skipping mod <name>: <reason> [api-bad-mod]   (the message format)
```

## api-bad-pbo

Raised by: `api` (also `api grep`), `mcp`.

A PBO of a mod is not readable as a PBO. It is skipped.

```text
In file …/badmod/addons/junk.pbo: warning: skipping unreadable PBO: …/badmod/addons/junk.pbo: failed to fill whole buffer [api-bad-pbo]
```

## api-cache

Raised by: `api`, `mcp`.

The cache folder cannot be found, created or written, or a cache file cannot be written. The query still runs on the freshly built index, only slower on the next run.

```text
warning: cannot create the cache directory /proc/nope/x: No such file or directory (os error 2); the index is built without a cache [api-cache]
```

## api-config-base-changed

Raised by: `api config`, `mcp`.

A class is defined again with another base class, or with none. The engine takes the base of the last definition, so a mod that writes `class X { ... }` without `: Base` cuts X off from its base. The lookup shows the change as well. Only a change made by a mod (a `-m` source) is warned about, whenever its PBO is merged over what loaded before it; the game changes some bases on purpose, and the lookup shows those changes without a warning.

```text
In file …/mod/addons/rebase.pbo: warning: class CfgVehicles/Hatch changes its base class: Base -> (none) [api-config-base-changed]
```

## api-config-include-unreadable

Raised by: `api config`, `mcp`.

A text `config.cpp` is indexed, and a text entry of the same PBO that it could include (`.hpp`, `.h`, `.inc`, `.txt`, `.cfg` and so on) is truncated or cannot be decompressed. The entry is skipped and the configs of the PBO still load. A config that includes the entry fails with the same reason (`api-bad-config`), not with "not found". PBOs with only a `config.bin` read no such entries.

```text
In file …/mod/addons/entry.pbo: warning: cannot read the includable zz_readme.txt: PBO entry "zz_readme.txt" is truncated (its data ends past the end of the file).; it is skipped [api-config-include-unreadable]
```

## api-config-includes-capped

Raised by: `api config`, `mcp`.

The includable text entries of one PBO add up to more than 64 MiB. The entries past that total are not read, once per PBO; a config that includes one of them fails with `api-bad-config`.

```text
In file …/mod/addons/huge.pbo: warning: the includable files of this PBO are more than 67108864 bytes; the rest is skipped [api-config-includes-capped]   (the message format; no synthetic file in this manual triggers it)
```

## api-config-limit

Raised by: `api config`, `mcp`.

The config of one source (the game or a mod) has more than 2 million classes or 10 million properties, or the merge of the selected sources does. The rest is skipped; the classes read so far can still be looked up.

```text
In file …/mod/addons/huge.pbo: warning: the config of mod is too large (more than 2000000 classes or 10000000 properties); the rest is skipped [api-config-limit]   (the message format; no synthetic file in this manual triggers it)
```

## api-config-order

Raised by: `api config`, `mcp`.

The `requiredAddons` of the `CfgPatches` of some PBOs depend on each other in a cycle (the PBOs can belong to different sources), so no order satisfies them. The PBOs of the cycle, and the PBOs that depend on them, load after the others, in the global tie-break order: the source (the game first, then the mods in `-m` order), then the path. Only the PBOs that really are in the cycle are named and counted, each as `[<source>] <pbo>`. The warning is raised on every query, not only when the index is built, because the merge runs for every query; the same holds for `api-config-base-changed` of a change across PBOs.

```text
warning: requiredAddons form a cycle between 2 configs ([vanilla] Addons/c1.pbo, [vanilla] Addons/c2.pbo); they and the configs that depend on them load last, in source and path order [api-config-order]
```

## api-unsafe-entry

Raised by: `api`, `mcp`.

A script entry of a PBO has a name that could not be a path (`..` components, a drive letter, control characters). It is skipped; no file system path is ever built from PBO content.

```text
In file …/badmod/addons/entry.pbo: warning: skipping unsafe entry name "..\x\a.c" [api-unsafe-entry]
```

## api-unsafe-prefix

Raised by: `api`, `mcp`.

The prefix in the header of a mod PBO is unsafe. The whole PBO is skipped.

```text
In file …/badmod/addons/unsafe.pbo: warning: skipping PBO with unsafe prefix "..\evil" [api-unsafe-prefix]
```

## binarize-missing-texture

Raised by: `build --proton-binarize`, `binarize`, and the MCP tools `pbo_build` and `binarize`.

`binarize.exe` printed a line that says it cannot find a texture or material (`cannot`, `not found`). Typical for references outside the addon: `binarize.exe` resolves paths relative to the addon only, so vanilla `dz\...` textures stay unresolved. The model is still binarized; only the alpha flags depend on reading the texture.

```text
warning: <model>: <the line binarize.exe printed> [binarize-missing-texture]   (needs DayZ Tools; not reproducible without it)
```

## derapify-failed

Raised by: `extract --derapify`, and the MCP tool `extract`.

A `*.bin` starts with the rapified signature but cannot be read as a config. It is written as it is, with its `.bin` name.

```text
warning: Keeping "config.bin" as it is: failed to fill whole buffer [derapify-failed]
```

## duplicate-entry

Raised by: `build`, `pack`, `unpack`, `extract`.

Two files map to the same PBO entry name ignoring case (`A.txt` and `a.txt`), or, when unpacking, a PBO has two entries with the same path. The first in sorted order is used and the other skipped. An entry generated from source (the rapified `config.bin`) wins over a plain file of the same name.

```text
warning: Skipping "data/a.txt": an entry named "data\A.txt" was already packed. [duplicate-entry]
```

## duplicate-path

Raised by: `extract`.

Two PBOs of the source write the same path ignoring case. The PBO that comes later in sorted order wins; the warning names both.

```text
warning: "a/x.txt" is in Addons/c.pbo and in dta/a.pbo; keeping the one of dta/a.pbo. [duplicate-path]
```

## empty-value

Raised by: `rapify`, `build`, `config_rapify`.

A config entry has no value (`z = ;`, or a macro that expanded to nothing). It is stored as an empty string.

```text
In file warn.cpp:1: warning: Entry "z" has no value; it is stored as an empty string. [empty-value]
```

## extract-path-clash

Raised by: `extract`, and the MCP tool `extract`.

An entry would be a file where an earlier entry of the same extract has a folder, or a folder where it has a file (`p/a` in one PBO, `p/a/b` in another). The first one in the sorted PBO order is written; the later entry is skipped and is not part of the tree. The warning says which case it is and names both PBOs and the path (always with `/`). The other case reads `Skipping "p/a" from <pbo>: it is a file, but <other pbo> has a folder there`; `<pbo>` is the skipped one and `<other pbo>` the one whose entry was written (the owner).

```text
warning: Skipping "p/x/y.txt" from addons/b.pbo: it needs "p/x" as a folder, but addons/a.pbo has a file there. [extract-path-clash]
```

## include-case

Raised by: `rapify`, `preprocess`, `build`.

An `#include` only matched a file when case is ignored. It works on Linux because dayzmake falls back to a case-insensitive match, but DayZ itself runs on Windows file names; fix the case so the build is the same everywhere.

```text
warning: Include "Inc/Common.HPP" only matched ".../w/inc/common.hpp" ignoring case. [include-case]
```

## leading-separator

Raised by: `unpack`, `extract`.

A PBO entry name starts with `\` or `/`. It is extracted without it.

```text
warning: Entry "\scripts\hello.c" starts with a path separator; extracting it as "scripts/hello.c". [leading-separator]
```

## macro-arg-count

Raised by: `rapify`, `preprocess`, `build`.

A macro is called with a different number of arguments than it declares.

```text
In file mac.cpp:2: warning: Macro "M" expects 2 argument(s), got 1 [macro-arg-count]
```

## missing-material

Raised by: `build`, `pbo_build`.

An editable model references a material (`.rvmat`) that starts with the addon's prefix but is not in the addon. The rules for other prefixes are those of [missing-texture](#missing-texture).

```text
In file small.p3d: warning: Model references material "t21\data\m.rvmat", which is not in the addon. [missing-material]
```

## missing-texture

Raised by: `build`, `pbo_build`.

An editable model references a texture that starts with the addon's prefix but is not in the addon. References to other prefixes are only checked if the file exists below an `-i` folder or the current directory; if not, nothing is reported, because there is no way to tell whether they are wrong. Procedural textures (`#(argb,...)`) are skipped.

```text
In file small.p3d: warning: Model references texture "t21\data\gone_co.paa", which is not in the addon. [missing-texture]
```

## model-unreadable

Raised by: `build`, `pbo_build`.

A `.p3d` is neither a readable MLOD nor a known binarized model, so its textures and materials cannot be checked.

```text
In file data/broken.p3d: warning: Cannot check textures and materials: not an MLOD model [model-unreadable]
```

## navmesh-seed

Raised by: `navmesh generate`.

The island filter found no polygon near the seed position (a box that reaches 30 m in x, 30 m in height and 3 m in z to each side of it), so it was skipped and no polygon is flagged as an island. The seed is the world's `seedPosition` or the one given with `--seed`; it has to be on land, near walkable ground.

```text
warning: island filter skipped: no polygon near the seed (-5000.0 10.0 -5000.0) [navmesh-seed]
```

## navmesh-stray-part

Raised by: `navmesh generate`.

A built tile had polygons whose door part id was beyond the list of parts of the tile. This happens for trees below sea level on a tile that also has doors. The ids were ignored (the polygons get no door part); the mesh is complete. One warning per tile.

```text
warning: tile (12,40): 3 polygons had out-of-range door part ids, which were ignored [navmesh-stray-part]
```

## navmesh-tile-failed

Raised by: `navmesh generate`.

A tile could not be built or added to the mesh, or its data from the server was malformed. The run goes on without that tile, saves the mesh and exits with status 1; the tile is listed in the report. One warning per tile.

```text
warning: tile (1,0): unknown cell class 99 [navmesh-tile-failed]
```

## non-utf8-filename

Raised by: `unpack`, `extract`.

A PBO entry name is not valid UTF-8. It is extracted as Windows-1252.

```text
warning: Entry name "café.txt" is not valid UTF-8; extracting it as windows-1252. [non-utf8-filename]
```

## non-windows-binarization

Raised by: `build` and the MCP tool `pbo_build`.

On Linux and macOS, `build` without `--proton-binarize` copies each `.p3d` and `.rtm` as it is.

```text
In file small.p3d: warning: On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is. [non-windows-binarization]
```

## prebinarized-file

Raised by: `build`, `binarize`, `pbo_build`.

A model or animation is already binarized (`ODOL`, `BMTR`). It is copied unchanged and `binarize.exe` is not run. The message names the format and version.

```text
In file data/odol.p3d: warning: File is already binarized (ODOL v58); copying it as-is. [prebinarized-file]
```

## old-tree-left

Raised by: `extract`.

The new tree is in place, but the old target folder that was moved away could not be removed. Delete the folder the warning names.

```text
warning: Could not remove the old tree ".out.extract-4242-0.old": Permission denied (os error 13) [old-tree-left]
```

## prefix-slash

Raised by: `build`, `pack`.

The prefix in `$PBOPREFIX$` or `-e prefix=...` has leading or trailing slashes. They are removed.

```text
warning: Removed leading/trailing slashes from prefix "/leading/". [prefix-slash]
```

## prerapified-file

Raised by: `rapify`, `build`.

A config that should be rapified already is. It is copied unchanged.

```text
In file data/pre.cpp: warning: File is already rapified; copying it as-is. [prerapified-file]
```

## unquoted-string

Raised by: `rapify`, `build`.

A value or array element is not in quotes (`name = hello;`). It is stored as a string. Macro arguments that are not evaluated show up here too.

```text
In file warn.cpp:1: warning: String value "hello" is not quoted properly. [unquoted-string]
```

## unsafe-header

Raised by: `unpack`.

A header extension of a PBO contains control characters (line breaks). They are replaced with spaces in the `$PBOPREFIX$` that `unpack` writes.

```text
warning: Header extension "note" contains control characters; replaced them with spaces. [unsafe-header]
```

## unsafe-path

Raised by: `build`, `pack`, `unpack`, `extract`.

A file name is not usable as a PBO entry name (not UTF-8, control characters, `:`, a Windows device name such as `CON` or `NUL`, `..`), or, when unpacking, an entry would leave the target folder or go through a symbolic link. The file or entry is skipped. `extract` also skips a whole PBO whose prefix is not a safe folder name or is `pbo-map.tsv`.

```text
warning: Skipping "data/bad:name.txt": unsafe entry name. [unsafe-path]
```

## unsupported-file

Raised by: `build`, `pack`.

A file in the source folder is a FIFO, a device or a socket. It is skipped; dayzmake never opens it, so it cannot hang on it.

```text
In file data/pipe: warning: Skipping a file that is not a regular file (FIFO, device or socket). [unsupported-file]
```

## unterminated-comment

Raised by: `rapify`, `preprocess`, `build`, `api config`, `mcp`.

A `/*` comment is never closed. Like CfgConvert, which says nothing about it, dayzmake takes the rest of the file as the comment, so every line after it is ignored, also an `#include` (which is then not even looked up). In an included file the comment ends with that file. The warning names the file and the line of the `/*`; it is not raised for `/*` inside a string. It also covers a comment that opens after a directive (`#endif /* c`, `#define X 1 /* c`), and one in a disabled `#ifdef` branch.

```text
In file open.cpp:2: warning: A /* comment is never closed; the rest of the file is ignored. [unterminated-comment]
```
