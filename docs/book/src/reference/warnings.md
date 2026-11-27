# Warnings

Every warning has a name. The name is in square brackets at the end of its line. To mute a warning, use `-w <name>`
on the commands that accept it (`rapify`, `preprocess`, `binarize`, `build`). You cannot mute the warnings of `pack`,
`unpack` and `api`. The MCP server returns the warnings with the result of each tool call. dayzmake shows at most ten
warnings of the same name in each run. Use `-v` to see them all. A warning never changes the exit status. Only errors
change it.

The messages below are real output (`…` marks a shortened path). A note marks the exceptions. In an exception, the
format is quoted from the source, because the situation needs files that this manual cannot include (DayZ Tools, a
damaged PBO of the game).

Each section lists the commands that can raise the warning, what triggers it and an example.

## api-bad-config

Raised by: `api`, `mcp`.

dayzmake cannot read a `config.bin` or `config.cpp`. dayzmake resolves an `#include` in a `config.cpp` only from the
entries of the same PBO. The path is relative to the including entry, or it is `\prefix\path` inside the prefix of the
PBO. These cases make the config unreadable: an absolute path, a drive letter, a `..` that leaves the root of the PBO,
and a file that is not in the PBO. (A `..` inside the PBO is resolved against the folder of the including entry.)
dayzmake never reads the file system for an include. For a mod PBO, dayzmake does not collect its `defines[]`, but it
still indexes the scripts of the PBO. For `api config`, dayzmake leaves the config out of the config index. dayzmake
skips a class with a name that cannot be a path segment (empty, with a `/` or a control character) with the same
warning.

```text
In file …/badmod/addons/entry.pbo: warning: cannot read config.bin: failed to fill whole buffer [api-bad-config]
```

## api-bad-entry

Raised by: `api`, `mcp`.

dayzmake cannot read an entry of a PBO that holds scripts (for example, a corrupt compressed entry). It skips the entry.

```text
warning: cannot read "<entry name>": <reason> [api-bad-entry]   (the message format; no synthetic file in this manual triggers it)
```

## api-bad-mod

Raised by: `api`, `mcp`.

dayzmake cannot index a selected mod. For example, its folder disappeared, or an indexing thread failed. dayzmake skips the mod and answers for the other mods.

```text
warning: skipping mod <name>: <reason> [api-bad-mod]   (the message format)
```

## api-bad-pbo

Raised by: `api` (also `api grep`), `mcp`.

dayzmake cannot read a PBO of a mod as a PBO. It skips the PBO.

```text
In file …/badmod/addons/junk.pbo: warning: skipping unreadable PBO: …/badmod/addons/junk.pbo: failed to fill whole buffer [api-bad-pbo]
```

## api-cache

Raised by: `api`, `mcp`.

dayzmake cannot find, create or write the cache folder, or it cannot write a cache file. The query still runs on the new index. The next run is slower.

```text
warning: cannot create the cache directory /proc/nope/x: No such file or directory (os error 2); the index is built without a cache [api-cache]
```

## api-config-base-changed

Raised by: `api config`, `mcp`.

A class is defined again with another base class, or with none. The engine takes the base of the last definition. A mod
that writes `class X { ... }` without `: Base` therefore cuts X off from its base. The lookup shows the change too.
dayzmake warns only about a change that a mod (a `-m` source) makes, each time it merges the PBO of the mod over what
loaded before it. The game changes some bases on purpose. The lookup shows those changes without a warning.

```text
In file …/mod/addons/rebase.pbo: warning: class CfgVehicles/Hatch changes its base class: Base -> (none) [api-config-base-changed]
```

## api-config-include-unreadable

Raised by: `api config`, `mcp`.

dayzmake indexes a text `config.cpp`, and a text entry of the same PBO that the config could include (`.hpp`, `.h`,
`.inc`, `.txt`, `.cfg` and so on) is truncated or cannot be decompressed. dayzmake skips the entry, and the configs of
the PBO still load. A config that includes the entry fails with the same reason (`api-bad-config`), not with "not
found". dayzmake reads no such entries for PBOs with only a `config.bin`.

```text
In file …/mod/addons/entry.pbo: warning: cannot read the includable zz_readme.txt: PBO entry "zz_readme.txt" is truncated (its data ends past the end of the file).; it is skipped [api-config-include-unreadable]
```

## api-config-includes-capped

Raised by: `api config`, `mcp`.

The includable text entries of one PBO add up to more than 64 MiB. dayzmake does not read the entries past that total. It warns once for each PBO. A config that includes one of these entries fails with `api-bad-config`.

```text
In file …/mod/addons/huge.pbo: warning: the includable files of this PBO are more than 67108864 bytes; the rest is skipped [api-config-includes-capped]   (the message format; no synthetic file in this manual triggers it)
```

## api-config-limit

Raised by: `api config`, `mcp`.

The config of one source (the game or a mod) has more than 2 million classes or 10 million properties. The merge of the selected sources can also exceed these limits. dayzmake skips the rest. You can still look up the classes that it read before.

```text
In file …/mod/addons/huge.pbo: warning: the config of mod is too large (more than 2000000 classes or 10000000 properties); the rest is skipped [api-config-limit]   (the message format; no synthetic file in this manual triggers it)
```

## api-config-order

Raised by: `api config`, `mcp`.

The `requiredAddons` of the `CfgPatches` of some PBOs depend on each other in a cycle (the PBOs can belong to different
sources), so no order satisfies them. The PBOs of the cycle, and the PBOs that depend on them, load after the others.
They load in the global tie-break order: first the source (the game first, then the mods in `-m` order), then the
path. The warning names and counts only the PBOs that really are in the cycle, each as `[<source>] <pbo>`. dayzmake
raises the warning on every query, not only when it builds the index, because the merge runs for every query. The
same is true for `api-config-base-changed` of a change across PBOs.

```text
warning: requiredAddons form a cycle between 2 configs ([vanilla] Addons/c1.pbo, [vanilla] Addons/c2.pbo); they and the configs that depend on them load last, in source and path order [api-config-order]
```

## api-unsafe-entry

Raised by: `api`, `mcp`.

A script entry of a PBO has a name that cannot be a path (`..` components, a drive letter, control characters). dayzmake skips the entry. It never builds a file system path from PBO content.

```text
In file …/badmod/addons/entry.pbo: warning: skipping unsafe entry name "..\x\a.c" [api-unsafe-entry]
```

## api-unsafe-prefix

Raised by: `api`, `mcp`.

The prefix in the header of a mod PBO is unsafe. dayzmake skips the whole PBO.

```text
In file …/badmod/addons/unsafe.pbo: warning: skipping PBO with unsafe prefix "..\evil" [api-unsafe-prefix]
```

## binarize-missing-texture

Raised by: `build --proton-binarize`, `binarize`, and the MCP tools `pbo_build` and `binarize`.

`binarize.exe` printed a line that says it cannot find a texture or material (`cannot`, `not found`). This is typical
for references outside the addon. `binarize.exe` resolves paths relative to the addon only, so vanilla `dz\...`
textures stay unresolved. `binarize.exe` still binarizes the model. Only the alpha flags depend on the texture.

```text
warning: <model>: <the line binarize.exe printed> [binarize-missing-texture]   (needs DayZ Tools; not reproducible without it)
```

## derapify-failed

Raised by: `extract --derapify`, and the MCP tool `extract`.

A `*.bin` starts with the rapified signature, but dayzmake cannot read it as a config. dayzmake writes it as it is, with its `.bin` name.

```text
warning: Keeping "config.bin" as it is: failed to fill whole buffer [derapify-failed]
```

## duplicate-entry

Raised by: `build`, `pack`, `unpack`, `extract`.

Two files map to the same PBO entry name when case is ignored (`A.txt` and `a.txt`), or, when you unpack, a PBO has two entries with the same path. dayzmake uses the first in sorted order and skips the other. An entry that dayzmake generates from source (the rapified `config.bin`) wins over a plain file of the same name.

```text
warning: Skipping "data/a.txt": an entry named "data\A.txt" was already packed. [duplicate-entry]
```

## duplicate-path

Raised by: `extract`.

Two PBOs of the source write the same path when case is ignored. The PBO that comes later in sorted order wins. The warning names both PBOs.

```text
warning: "a/x.txt" is in Addons/c.pbo and in dta/a.pbo; keeping the one of dta/a.pbo. [duplicate-path]
```

## empty-value

Raised by: `rapify`, `build`, `config_rapify`.

A config entry has no value (`z = ;`, or a macro that expanded to nothing). dayzmake stores it as an empty string.

```text
In file warn.cpp:1: warning: Entry "z" has no value; it is stored as an empty string. [empty-value]
```

## extract-path-clash

Raised by: `extract`, and the MCP tool `extract`.

An entry would be a file where an earlier entry of the same extract has a folder, or a folder where it has a file
(`p/a` in one PBO, `p/a/b` in another). dayzmake writes the first one in the sorted PBO order. It skips the later
entry, and the entry is not part of the tree. The warning says which case it is. It names both PBOs and the path
(always with `/`). The other case reads `Skipping "p/a" from <pbo>: it is a file, but <other pbo> has a folder there`.
`<pbo>` is the skipped PBO. `<other pbo>` is the PBO whose entry dayzmake wrote (the owner).

```text
warning: Skipping "p/x/y.txt" from addons/b.pbo: it needs "p/x" as a folder, but addons/a.pbo has a file there. [extract-path-clash]
```

## include-case

Raised by: `rapify`, `preprocess`, `build`.

An `#include` matched a file only when case is ignored. It works on Linux, because dayzmake falls back to a case-insensitive match. DayZ itself uses Windows file names. Fix the case, so that the build is the same everywhere.

```text
warning: Include "Inc/Common.HPP" only matched ".../w/inc/common.hpp" ignoring case. [include-case]
```

## leading-separator

Raised by: `unpack`, `extract`.

A PBO entry name starts with `\` or `/`. dayzmake extracts it without this character.

```text
warning: Entry "\scripts\hello.c" starts with a path separator; extracting it as "scripts/hello.c". [leading-separator]
```

## macro-arg-count

Raised by: `rapify`, `preprocess`, `build`.

A call of a macro has a different number of arguments than the macro declares.

```text
In file mac.cpp:2: warning: Macro "M" expects 2 argument(s), got 1 [macro-arg-count]
```

## missing-material

Raised by: `build`, `pbo_build`.

An editable model references a material (`.rvmat`) that starts with the prefix of the addon, but the material is not in the addon. The rules for other prefixes are the same as for [missing-texture](#missing-texture).

```text
In file small.p3d: warning: Model references material "t21\data\m.rvmat", which is not in the addon. [missing-material]
```

## missing-texture

Raised by: `build`, `pbo_build`.

An editable model references a texture that starts with the prefix of the addon, but the texture is not in the addon. dayzmake checks references to other prefixes only if the file exists below an `-i` folder or the current directory. If the file does not exist, dayzmake reports nothing, because it cannot know whether the reference is wrong. dayzmake skips procedural textures (`#(argb,...)`).

```text
In file small.p3d: warning: Model references texture "t21\data\gone_co.paa", which is not in the addon. [missing-texture]
```

## model-unreadable

Raised by: `build`, `pbo_build`.

A `.p3d` is neither a readable MLOD nor a known binarized model, so dayzmake cannot check its textures and materials.

```text
In file data/broken.p3d: warning: Cannot check textures and materials: not an MLOD model [model-unreadable]
```

## navmesh-seed

Raised by: `navmesh generate`, `navmesh_generate`.

The island filter found no polygon near the seed position (a box that reaches 30 m in x, 30 m in height and 3 m in z to each side of it). dayzmake skipped the filter, so no polygon is flagged as an island. The seed is the `seedPosition` of the world, or the position that you give with `--seed`. It must be on land, near walkable ground.

```text
warning: island filter skipped: no polygon near the seed (-5000.0 10.0 -5000.0) [navmesh-seed]
```

## navmesh-stray-part

Raised by: `navmesh generate`, `navmesh_generate`.

A built tile had polygons whose door part id was beyond the list of parts of the tile. This happens for trees below sea level on a tile that also has doors. dayzmake ignored the ids (the polygons get no door part). The mesh is complete. dayzmake gives one warning for each tile.

```text
warning: tile (12,40): 3 polygons had out-of-range door part ids, which were ignored [navmesh-stray-part]
```

## navmesh-tile-failed

Raised by: `navmesh generate`, `navmesh_generate`.

dayzmake could not build a tile or add it to the mesh, or the data of the tile from the server was malformed. The run goes on without that tile, saves the mesh and exits with status 1. The report lists the tile. dayzmake gives one warning for each tile.

```text
warning: tile (1,0): unknown cell class 99 [navmesh-tile-failed]
```

## non-utf8-filename

Raised by: `unpack`, `extract`.

A PBO entry name is not valid UTF-8. dayzmake extracts it as Windows-1252.

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

A model or animation is already binarized (`ODOL`, `BMTR`). dayzmake copies it unchanged and does not run `binarize.exe`. The message names the format and version.

```text
In file data/odol.p3d: warning: File is already binarized (ODOL v58); copying it as-is. [prebinarized-file]
```

## old-tree-left

Raised by: `extract`.

The new tree is in place, but dayzmake could not remove the old target folder that it moved away. Delete the folder that the warning names.

```text
warning: Could not remove the old tree ".out.extract-4242-0.old": Permission denied (os error 13) [old-tree-left]
```

## prefix-slash

Raised by: `build`, `pack`.

The prefix in `$PBOPREFIX$` or `-e prefix=...` has leading or trailing slashes. dayzmake removes them.

```text
warning: Removed leading/trailing slashes from prefix "/leading/". [prefix-slash]
```

## prerapified-file

Raised by: `rapify`, `build`.

A config that dayzmake must rapify is already rapified. dayzmake copies it unchanged.

```text
In file data/pre.cpp: warning: File is already rapified; copying it as-is. [prerapified-file]
```

## unquoted-string

Raised by: `rapify`, `build`.

A value or array element is not in quotes (`name = hello;`). dayzmake stores it as a string. Macro arguments that dayzmake does not evaluate also cause this warning.

```text
In file warn.cpp:1: warning: String value "hello" is not quoted properly. [unquoted-string]
```

## unsafe-header

Raised by: `unpack`.

A header extension of a PBO contains control characters (line breaks). `unpack` replaces them with spaces in the `$PBOPREFIX$` that it writes.

```text
warning: Header extension "note" contains control characters; replaced them with spaces. [unsafe-header]
```

## unsafe-path

Raised by: `build`, `pack`, `unpack`, `extract`.

A file name cannot be a PBO entry name (not UTF-8, control characters, `:`, a Windows device name such as `CON` or `NUL`, `..`). When you unpack, an entry can also leave the target folder or go through a symbolic link. dayzmake skips the file or entry. `extract` also skips a whole PBO whose prefix is not a safe folder name or is `pbo-map.tsv`.

```text
warning: Skipping "data/bad:name.txt": unsafe entry name. [unsafe-path]
```

## unsupported-file

Raised by: `build`, `pack`.

A file in the source folder is a FIFO, a device or a socket. dayzmake skips it. It never opens the file, so it cannot hang on it.

```text
In file data/pipe: warning: Skipping a file that is not a regular file (FIFO, device or socket). [unsupported-file]
```

## unterminated-comment

Raised by: `rapify`, `preprocess`, `build`, `api config`, `mcp`.

A `/*` comment is never closed. CfgConvert says nothing about this. dayzmake does the same, and takes the rest of the
file as the comment. It ignores every line after the comment, also an `#include` (which it does not look up). In an
included file, the comment ends with that file. The warning names the file and the line of the `/*`. dayzmake does
not raise it for a `/*` inside a string. The warning also covers a comment that opens after a directive
(`#endif /* c`, `#define X 1 /* c`), and a comment in a disabled `#ifdef` branch.

```text
In file open.cpp:2: warning: A /* comment is never closed; the rest of the file is ignored. [unterminated-comment]
```
