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

A mod PBO has a `config.bin` that cannot be read, so its `defines[]` are not collected. The scripts of the PBO are still indexed.

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

Raised by: `api`, `mcp`.

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

## duplicate-entry

Raised by: `build`, `pack`, `unpack`.

Two files map to the same PBO entry name ignoring case (`A.txt` and `a.txt`), or, when unpacking, a PBO has two entries with the same path. The first in sorted order is used and the other skipped. An entry generated from source (the rapified `config.bin`) wins over a plain file of the same name.

```text
warning: Skipping "data/a.txt": an entry named "data\A.txt" was already packed. [duplicate-entry]
```

## empty-value

Raised by: `rapify`, `build`, `config_rapify`.

A config entry has no value (`z = ;`, or a macro that expanded to nothing). It is stored as an empty string.

```text
In file warn.cpp:1: warning: Entry "z" has no value; it is stored as an empty string. [empty-value]
```

## include-case

Raised by: `rapify`, `preprocess`, `build`.

An `#include` only matched a file when case is ignored. It works on Linux because dayzmake falls back to a case-insensitive match, but DayZ itself runs on Windows file names; fix the case so the build is the same everywhere.

```text
warning: Include "Inc/Common.HPP" only matched ".../w/inc/common.hpp" ignoring case. [include-case]
```

## leading-separator

Raised by: `unpack`.

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

## non-utf8-filename

Raised by: `unpack`.

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

Raised by: `build`, `pack`, `unpack`.

A file name is not usable as a PBO entry name (not UTF-8, control characters, `:`, a Windows device name such as `CON` or `NUL`, `..`), or, when unpacking, an entry would leave the target folder or go through a symbolic link. The file or entry is skipped.

```text
warning: Skipping "data/bad:name.txt": unsafe entry name. [unsafe-path]
```

## unsupported-file

Raised by: `build`, `pack`.

A file in the source folder is a FIFO, a device or a socket. It is skipped; dayzmake never opens it, so it cannot hang on it.

```text
In file data/pipe: warning: Skipping a file that is not a regular file (FIFO, device or socket). [unsupported-file]
```

